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
use std::{collections::HashMap, fs, process::Command};

/// Observations per `evidence.batch`.
const BATCH: usize = 100;

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

/// The extractor's own vocabulary: a namespace it lists among its dependencies.
fn vocabulary(plugin: &Plugin) -> Result<Vocabulary> {
    let file = plugin
        .manifest
        .vocabulary
        .as_deref()
        .context("an extractor names its vocabulary file")?;
    let vocabulary = Vocabulary::parse(&fs::read_to_string(plugin.dir.join(file))?)?;
    ensure!(
        plugin.manifest.dependencies.contains(&vocabulary.namespace),
        "vocabulary {} isn't among the dependencies",
        vocabulary.namespace
    );
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
    let Ok(mut process) = Connector::launch(command, &manifest.id, Limits::from_env()) else {
        return Err("not_installed");
    };
    let outcome =
        converse(storage, &mut process.wire, &process.relay, run, counts).unwrap_or_else(|error| {
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
    run: &Run,
    counts: &mut Counts,
) -> Result<Outcome> {
    let manifest = &run.plugin.manifest;
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
    let mut accepted: Vec<ObservedClaim> = Vec::new();
    loop {
        let message = match wire.recv() {
            Ok(message) => message,
            Err(reason) => return Ok(Err(reason)),
        };
        match message.method.as_deref() {
            Some(CLAIM_SUBMIT) => {
                counts.submitted += 1;
                let checked = message
                    .params()
                    .map_err(|error| format!("{error:#}"))
                    .and_then(|submit| {
                        claims::validate(submit, &delivered, manifest, run.vocabulary)
                    });
                match checked {
                    Ok(claim) => accepted.push(claim),
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
                    });
                    let stored = storage.record_claims(
                        run.run_id,
                        (&manifest.id, &manifest.version),
                        &accepted,
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
                if let Err(reason) = wire.send(&Envelope::response(message.id, *counts)?) {
                    return Ok(Err(reason));
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
