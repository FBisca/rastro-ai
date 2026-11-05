//! Running a source's bound extractor over its pending evidence (ADR 0008).
use super::{Connector, Limits, Plugin, Wire, allowed_environment, discover, handshake, resolve};
use crate::claims::{self, Checkpoint, ObservedClaim};
use crate::runs::{ExtractionReport, ExtractionState};
use crate::{Counts, Evidence, Locations, RunEnd, Source, Storage};
use anyhow::{Context, Result, ensure};
use chrono::{SecondsFormat, Utc};
use rastro_plugin_sdk::{
    BatchItem, CLAIM_SUBMIT, COULDNT_LOOK_REASONS, EVIDENCE_BATCH, Envelope, EvidenceBatch,
    ExtractionBinding, ExtractorInitializeParams, PROTOCOL_VERSION, RUN_FINISH, Role, RunFinish,
    SourceScope, Span, Vocabulary,
};
use std::{
    collections::{BTreeMap, HashMap, btree_map::Entry},
    fs,
    path::{Component, Path},
    process::Command,
    time::Instant,
};

/// Observations per `evidence.batch`.
const BATCH: usize = 100;
/// Claims a run may submit unless the limits say otherwise: this many per observation
/// delivered, and never fewer than `MIN_CLAIMS`.
const CLAIMS_PER_EVIDENCE: usize = 20;
const MIN_CLAIMS: usize = 1000;

type Outcome = std::result::Result<(), &'static str>;

/// Runs `extractor` over the source's evidence past its checkpoint and logs the run. Accepted
/// claims and the new checkpoint are stored together when the extractor finishes cleanly;
/// otherwise nothing is, and the next sync starts from the same checkpoint.
pub fn extract<S: Storage>(
    storage: &mut S,
    locations: &Locations,
    source: &Source,
    extractor: &str,
) -> Result<ExtractionReport> {
    let found =
        discover(locations, extractor, "extractor").and_then(|plugin| match vocabulary(&plugin) {
            Ok(vocabulary) => Ok((plugin, vocabulary)),
            Err(error) => {
                eprintln!("plugin {extractor} fails its check: {error:#}");
                Err("protocol")
            }
        });
    let version = found
        .as_ref()
        .map_or_else(|_| String::new(), |(p, _)| p.manifest.version.clone());
    let run_id = storage.start_extraction(source, (extractor, &version), Utc::now())?;
    let mut counts = Counts::default();
    let mut evidence = 0;
    let outcome = match &found {
        Err(reason) => Err(*reason),
        Ok((plugin, _)) if !plugin.manifest.permissions.scopes.contains(&source.scope.0) => {
            Err("forbidden")
        }
        Ok((plugin, vocabulary)) => {
            let pending = storage.pending_evidence((extractor, &version), source)?;
            evidence = pending.len();
            let run = Run {
                plugin,
                vocabulary,
                source,
                run_id,
                pending,
            };
            drive(storage, &run, &mut counts)
        }
    };
    let reason = outcome.err().map(String::from);
    storage.finish_run(
        run_id,
        &RunEnd {
            status: if reason.is_some() { "failed" } else { "ok" }.into(),
            reason: reason.clone(),
            finished_at: Utc::now(),
            counts,
        },
    )?;
    let state = match reason.as_deref() {
        None => ExtractionState::Current,
        Some("not_installed") => ExtractionState::NotInstalled,
        Some(_) => ExtractionState::Failed,
    };
    Ok(ExtractionReport {
        run_id,
        extractor: extractor.into(),
        extractor_version: version,
        state,
        reason,
        evidence,
        counts,
    })
}

/// The extractor's own vocabulary: a file inside its directory, defining a namespace it lists
/// among its dependencies, every type and predicate named in that namespace.
fn vocabulary(plugin: &Plugin) -> Result<Vocabulary> {
    let file = plugin
        .manifest
        .vocabulary
        .as_deref()
        .context("an extractor names its vocabulary file")?;
    ensure!(
        Path::new(file)
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "the vocabulary file must lie inside the plugin directory"
    );
    let vocabulary = Vocabulary::parse(&fs::read_to_string(plugin.dir.join(file))?)?;
    let namespace = &vocabulary.namespace;
    ensure!(
        plugin.manifest.dependencies.contains(namespace),
        "vocabulary {namespace} isn't among the dependencies"
    );
    let prefix = format!("{namespace}:");
    let ids = vocabulary.types.iter();
    let ids = ids.chain(vocabulary.predicates.iter().map(|p| &p.id));
    for id in ids {
        ensure!(
            id.len() > prefix.len() && id.starts_with(&prefix),
            "vocabulary {namespace} defines {id} outside its namespace"
        );
    }
    Ok(vocabulary)
}

/// What one extractor run reads and may claim from.
struct Run<'a> {
    plugin: &'a Plugin,
    vocabulary: &'a Vocabulary,
    source: &'a Source,
    run_id: i64,
    pending: Vec<(i64, Evidence)>,
}

/// Spawns the extractor and talks to it until `run.finish`. A storage error fails the run as
/// `storage`.
fn drive<S: Storage>(storage: &mut S, run: &Run, counts: &mut Counts) -> Outcome {
    let manifest = &run.plugin.manifest;
    let mut command = Command::new(resolve(&run.plugin.dir, &manifest.command));
    command.env_clear().envs(allowed_environment(manifest));
    // Discovery found the manifest; a command that won't start is the plugin's fault.
    let Ok(mut process) = Connector::launch(command, &manifest.id, Limits::from_env()) else {
        return Err("protocol");
    };
    let wire = &mut process.wire;
    let outcome = converse(storage, wire, &process.relay, &process.limits, run, counts)
        .unwrap_or_else(|error| {
            process.relay.line(&format!("storage error: {error:#}"));
            Err("storage")
        });
    process.teardown(outcome.is_ok());
    outcome
}

fn converse<S: Storage>(
    storage: &mut S,
    wire: &mut Wire,
    relay: &super::io::Relay,
    limits: &Limits,
    run: &Run,
    counts: &mut Counts,
) -> Result<Outcome> {
    let manifest = &run.plugin.manifest;
    let end = Instant::now() + limits.extraction_time;
    let cap = limits
        .max_claims
        .unwrap_or(MIN_CLAIMS.max(CLAIMS_PER_EVIDENCE * run.pending.len()));
    let init = ExtractorInitializeParams {
        protocol: PROTOCOL_VERSION.into(),
        manifest_id: manifest.id.clone(),
        role: Role::Extractor,
        run: ExtractionBinding {
            run_id: run.run_id,
            extractor_version: manifest.version.clone(),
            pairs: vec![SourceScope {
                source: run.source.name.clone(),
                scope: run.source.scope.0.clone(),
            }],
        },
    };
    if let Err(reason) = handshake(wire, run.plugin, init)? {
        return Ok(Err(reason));
    }
    // Every batch is queued at once; the extractor's claims are read while it takes them in.
    let items: Vec<_> = run.pending.iter().map(item).collect();
    let mut batches: Vec<_> = items.chunks(BATCH).collect();
    if batches.is_empty() {
        batches.push(&[]);
    }
    let last = batches.len() - 1;
    for (index, batch) in batches.into_iter().enumerate() {
        let batch = EvidenceBatch {
            items: batch.to_vec(),
            last: index == last,
        };
        if let Err(reason) = wire.post(&Envelope::notification(EVIDENCE_BATCH, batch)?) {
            return Ok(Err(reason));
        }
    }
    let delivered: HashMap<i64, Evidence> = run.pending.iter().cloned().collect();
    // One entry per claim id; a repeat in the same run adds its supports and counts unchanged.
    let mut accepted: BTreeMap<String, ObservedClaim> = BTreeMap::new();
    loop {
        let message = match wire.recv_by(end) {
            Ok(message) => message,
            Err(reason) => return Ok(Err(reason)),
        };
        match message.method.as_deref() {
            Some(CLAIM_SUBMIT) => {
                counts.submitted += 1;
                if counts.submitted as usize > cap {
                    relay.line(&format!("more than {cap} claims in one run"));
                    return Ok(Err("protocol"));
                }
                let checked = message
                    .params()
                    .map_err(|error| format!("{error:#}"))
                    .and_then(|submit| {
                        claims::validate(submit, &delivered, manifest, run.vocabulary)
                    });
                match checked {
                    Ok(claim) => match accepted.entry(claim.id.clone()) {
                        Entry::Vacant(entry) => {
                            entry.insert(claim);
                        }
                        Entry::Occupied(mut entry) => {
                            entry.get_mut().supports.extend(claim.supports);
                            counts.unchanged += 1;
                        }
                    },
                    Err(why) => {
                        relay.line(&format!("rejected claim: {why}"));
                        counts.rejected += 1;
                    }
                }
            }
            Some(RUN_FINISH) if message.id.is_some() => {
                let Ok(finish) = message.params::<RunFinish>() else {
                    return Ok(Err("protocol"));
                };
                let outcome = if finish.status == "ok" {
                    let checkpoint = run.pending.last().map(|(id, e)| Checkpoint {
                        extractor: manifest.id.clone(),
                        extractor_version: manifest.version.clone(),
                        source: run.source.name.clone(),
                        scope: run.source.scope.clone(),
                        last: (e.captured_at, *id),
                        vocabulary_version: run.vocabulary.version.clone(),
                    });
                    let claims: Vec<ObservedClaim> =
                        std::mem::take(&mut accepted).into_values().collect();
                    let stored = storage.record_claims(
                        run.run_id,
                        (&manifest.id, &manifest.version),
                        &claims,
                        checkpoint.as_ref(),
                    )?;
                    counts.accepted += stored.accepted;
                    counts.unchanged += stored.unchanged;
                    counts.rejected += stored.rejected;
                    Ok(())
                } else {
                    // Nothing of a run its extractor calls failed is kept.
                    let reason = finish.reason.as_deref();
                    Err(COULDNT_LOOK_REASONS
                        .into_iter()
                        .find(|r| Some(*r) == reason)
                        .unwrap_or("protocol"))
                };
                // What was stored stays stored, even if the extractor can't hear the answer.
                if let Err(reason) = wire.send(&Envelope::response(message.id, *counts)?) {
                    relay.line(&format!("couldn't answer run.finish: {reason}"));
                }
                return Ok(outcome);
            }
            _ => return Ok(Err("protocol")),
        }
    }
}

fn item((id, e): &(i64, Evidence)) -> BatchItem {
    BatchItem {
        evidence_id: *id,
        source: e.source.clone(),
        scope: e.scope.0.clone(),
        location: e.location.clone(),
        span: Span {
            field: e.field.clone(),
            start: e.start,
            end: e.end,
            quote: e.quote.clone(),
        },
        captured_at: e.captured_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        source_updated_at: e
            .source_updated_at
            .map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
        trust_class: e.trust_class.as_str().into(),
    }
}
