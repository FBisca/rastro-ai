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
    collections::{BTreeSet, HashMap},
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
            let pending = storage.pending_evidence_for_vocabulary(
                (extractor, &version),
                &vocabulary.version,
                source,
            )?;
            evidence = pending.len();
            let context = context(storage, source, &pending)?;
            let run = Run {
                plugin,
                vocabulary,
                source,
                run_id,
                context,
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

/// Companion fields delivered at most per pending observation, so context stays bounded.
const CONTEXT_PER_EVIDENCE: usize = 4;

/// Location context: for the records the pending evidence changes, the other fields of the
/// same source, scope and location, as they stood at the batch's high-water mark (its last
/// observation). Fields that have pending evidence arrive as that evidence. Bounded: past
/// `CONTEXT_PER_EVIDENCE` per pending observation only the observations nearest the mark are
/// kept. The set is a function of the stored evidence and the mark, so a replay delivers the
/// same one.
fn context<S: Storage>(
    storage: &S,
    source: &Source,
    pending: &[(i64, Evidence)],
) -> Result<Vec<(i64, Evidence)>> {
    let Some((_, e)) = pending.last() else {
        return Ok(vec![]);
    };
    // The mark is the checkpoint's: the newest time and the highest local id of the batch.
    let mark = pending.iter().map(|(id, _)| *id).max().unwrap_or_default();
    let at: BTreeSet<&str> = pending.iter().map(|(_, e)| &*e.location).collect();
    let at: Vec<String> = at.into_iter().map(String::from).collect();
    let changed: BTreeSet<(&str, &str)> = pending
        .iter()
        .map(|(_, e)| (&*e.location, &*e.field))
        .collect();
    let mut context = storage.location_context(source, &at, (e.captured_at, mark))?;
    context.retain(|(_, e)| !changed.contains(&(&*e.location, &*e.field)));
    let keep = CONTEXT_PER_EVIDENCE * pending.len();
    context.drain(..context.len().saturating_sub(keep));
    Ok(context)
}

/// The extractor's own vocabulary: a file inside its directory, defining a namespace it lists
/// among its dependencies, every type and predicate named in that namespace.
pub(crate) fn vocabulary(plugin: &Plugin) -> Result<Vocabulary> {
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
    let ids = vocabulary.types.iter().map(|t| &t.id);
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
    /// What was already known at the locations with pending evidence: the latest observation
    /// up to the checkpoint of each field there that has no pending one. A field re-observed
    /// unchanged adds no evidence, so without this an extractor would see a changed login
    /// without the account id beside it. It is delivered, ahead of the pending evidence, like
    /// any other.
    context: Vec<(i64, Evidence)>,
    pending: Vec<(i64, Evidence)>,
}
impl Run<'_> {
    /// Everything delivered, in replay order: the context precedes the checkpoint, the
    /// pending evidence follows it.
    fn delivered(&self) -> impl Iterator<Item = &(i64, Evidence)> {
        self.context.iter().chain(&self.pending)
    }
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
        .unwrap_or(MIN_CLAIMS.max(CLAIMS_PER_EVIDENCE * run.delivered().count()));
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
    let items: Vec<_> = run.delivered().map(item).collect();
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
    let delivered: HashMap<i64, Evidence> = run.delivered().cloned().collect();
    // Preserve each submission as a separate AND derivation.
    let mut accepted: Vec<ObservedClaim> = Vec::new();
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
                    let checkpoint = checkpoint_boundary(&run.pending).map(|(id, e)| Checkpoint {
                        extractor: manifest.id.clone(),
                        extractor_version: manifest.version.clone(),
                        source: run.source.name.clone(),
                        scope: run.source.scope.clone(),
                        last: (e.captured_at, id),
                        vocabulary_version: run.vocabulary.version.clone(),
                        last_portable_key: claims::portable_evidence_key(e),
                    });
                    // The submission is validated together: an endpoint's type comes from a
                    // `rastro:type` claim in it, never from what an earlier run stored.
                    let (claims, ungrounded) = claims::ground(std::mem::take(&mut accepted));
                    counts.rejected += ungrounded;
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

fn checkpoint_boundary(pending: &[(i64, Evidence)]) -> Option<(i64, &Evidence)> {
    pending
        .iter()
        .max_by_key(|(id, _)| *id)
        .map(|(id, evidence)| (*id, evidence))
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

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    #[test]
    fn S3_checkpoint_boundary_names_one_append_even_when_capture_order_differs() {
        let observation = |captured: &str| Evidence {
            source: "source".into(),
            scope: crate::Scope("personal".into()),
            location: "https://example.org/item".into(),
            field: "/state".into(),
            start: 0,
            end: 4,
            quote: "open".into(),
            trust_class: crate::TrustClass::Structured,
            captured_at: captured.parse().unwrap(),
            source_updated_at: None,
            original: None,
        };
        let later_append = observation("2026-11-01T00:00:00Z");
        let pending = vec![
            (20, later_append.clone()),
            (10, observation("2026-11-02T00:00:00Z")),
        ];
        let (id, boundary) = checkpoint_boundary(&pending).unwrap();
        assert_eq!(id, 20);
        assert_eq!(boundary.captured_at, later_append.captured_at);
        assert_eq!(
            claims::portable_evidence_key(boundary),
            claims::portable_evidence_key(&later_append)
        );
    }
}
