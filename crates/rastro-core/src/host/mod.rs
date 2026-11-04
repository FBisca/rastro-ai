//! The plugin host: finds a connector, spawns it, and ingests what it submits (ADR 0003).
//! Sequential and happy-path first; hardening against crashes and silence builds on `drive`.
use crate::evidence::{Grant, validate};
use crate::runs::RunReport;
use crate::{
    Counts, CoverageRecord, CoverageState, Locations, RunEnd, Source, Storage, display_safe,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rastro_plugin_sdk::{
    COVERAGE_REPORT, CoverageReport, CoverageStatus, EVIDENCE_SUBMIT, Envelope, INITIALIZE,
    InitializeParams, InitializeResult, Manifest, PROTOCOL_VERSION, RUN_FINISH, RunBinding,
    RunFinish, Transport, Window, major,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

/// Finds the manifest whose `id` is the source's connector, under `<config>/plugins` and
/// the directories of `RASTRO_PLUGIN_PATH`. Each root may be a plugin directory or hold some.
/// `None` when no manifest matches: the connector isn't installed.
pub fn discover(locations: &Locations, connector: &str) -> Result<Option<Plugin>> {
    let mut roots = vec![locations.plugins()];
    if let Some(path) = env::var_os("RASTRO_PLUGIN_PATH") {
        roots.extend(env::split_paths(&path));
    }
    for root in roots {
        let mut candidates = vec![root.clone()];
        if let Ok(entries) = fs::read_dir(&root) {
            candidates.extend(entries.flatten().map(|e| e.path()));
        }
        for dir in candidates {
            let Ok(text) = fs::read_to_string(dir.join("rastro-plugin.toml")) else {
                continue;
            };
            match Manifest::parse(&text) {
                Ok(manifest) if manifest.id == connector => return check(manifest, dir).map(Some),
                Ok(_) => {}
                Err(error) => eprintln!("skipping {}: {error:#}", dir.display()),
            }
        }
    }
    Ok(None)
}

fn check(manifest: Manifest, dir: PathBuf) -> Result<Plugin> {
    if major(PROTOCOL_VERSION) != manifest.protocol.to_string() {
        bail!(
            "plugin {} speaks protocol {}",
            manifest.id,
            manifest.protocol
        );
    }
    if !manifest.kinds.iter().any(|k| k == "connector") {
        bail!("plugin {} is not a connector", manifest.id);
    }
    if !manifest.permissions.actions.is_empty() {
        bail!("a connector can't hold actions (plugin {})", manifest.id);
    }
    Ok(Plugin { manifest, dir })
}

fn stamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// What the conversation produced besides storage writes.
#[derive(Default)]
struct Session {
    counts: Counts,
    coverage: Option<CoverageReport>,
}

/// Runs one connector over `[from, to)` and records the run and its coverage.
pub fn run<S: Storage>(
    storage: &mut S,
    source: &Source,
    plugin: Option<&Plugin>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<RunReport> {
    let (plugin_id, plugin_version) = match plugin {
        Some(p) => (p.manifest.id.clone(), p.manifest.version.clone()),
        None => (source.connector.clone(), String::new()),
    };
    let started_at = Utc::now();
    let run_id = storage.start_run(source, (&plugin_id, &plugin_version), started_at)?;
    let window = Window {
        from: stamp(from),
        to: stamp(to),
    };
    let binding = RunBinding {
        run_id,
        source: source.name.clone(),
        scope: source.scope.0.clone(),
        selector: source.selector.clone(),
        window: window.clone(),
    };
    let mut session = Session::default();
    let outcome = match plugin {
        Some(plugin) => match Grant::new(&plugin.manifest, &source.name, &source.scope, &window) {
            Some(grant) => drive(storage, plugin, &binding, &grant, &mut session),
            None => Ok(Err("forbidden")),
        },
        None => Ok(Err("not_installed")),
    };
    let (status, reason) = match outcome {
        Ok(Ok(finish)) if finish.status == "ok" => ("ok", None),
        Ok(Ok(finish)) => ("failed", finish.reason.or(Some("plugin_failed".into()))),
        Ok(Err(reason)) => ("failed", Some(reason.to_string())),
        Err(error) => ("failed", Some(format!("storage: {error:#}"))),
    };
    let Session { counts, coverage } = session;
    // The connector reports completeness; the core decides found or empty.
    let (state, coverage_reason, retry_at, selection) = match coverage {
        Some(report) if report.status == CoverageStatus::Complete && status == "ok" => {
            let state = if counts.accepted > 0 {
                CoverageState::Found
            } else {
                CoverageState::Empty
            };
            (state, None, None, report.selection)
        }
        Some(report) => (
            CoverageState::CouldNotLook,
            report.reason.or(reason.clone()),
            report.retry_at.and_then(|t| t.parse().ok()),
            report.selection,
        ),
        None => (
            CoverageState::CouldNotLook,
            reason.clone().or(Some("no_coverage_report".into())),
            None,
            None,
        ),
    };
    let finished_at = Utc::now();
    storage.record_coverage(&CoverageRecord {
        run_id,
        source: source.name.clone(),
        scope: source.scope.clone(),
        window: (from, to),
        selection: selection.unwrap_or_else(|| "all".into()),
        state,
        reason: coverage_reason,
        retry_at,
        checked_at: finished_at,
    })?;
    storage.finish_run(
        run_id,
        &RunEnd {
            status: status.into(),
            reason: reason.clone(),
            finished_at,
            counts,
        },
    )?;
    Ok(RunReport {
        run_id,
        source: source.name.clone(),
        scope: source.scope.0.clone(),
        plugin_id,
        plugin_version,
        status: status.into(),
        reason,
        started_at,
        finished_at,
        window,
        counts,
        coverage: state,
    })
}

/// Spawns the connector and talks to it until `run.finish`. `Ok(Err(reason))` is a failed run;
/// `Err` is a storage error.
fn drive<S: Storage>(
    storage: &mut S,
    plugin: &Plugin,
    binding: &RunBinding,
    grant: &Grant,
    session: &mut Session,
) -> Result<std::result::Result<RunFinish, &'static str>> {
    let command = resolve(&plugin.dir, &plugin.manifest.command);
    let Ok(mut child) = Command::new(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .env_clear()
        .envs(allowed_environment(&plugin.manifest))
        .spawn()
    else {
        return Ok(Err("spawn"));
    };
    let stdin = child.stdin.take().context("child stdin")?;
    let stdout = child.stdout.take().context("child stdout")?;
    let mut transport = Transport::new(BufReader::new(stdout), stdin);
    let outcome = converse(storage, &mut transport, plugin, binding, grant, session);
    if !matches!(outcome, Ok(Ok(_))) {
        let _ = child.kill();
    }
    drop(transport);
    let _ = child.wait();
    outcome
}

/// The only variables a plugin starts with: the basics plus those its manifest declares.
fn allowed_environment(manifest: &Manifest) -> Vec<(OsString, OsString)> {
    env::vars_os()
        .filter(|(name, _)| {
            let name = name.to_string_lossy();
            matches!(name.as_ref(), "PATH" | "HOME" | "TMPDIR" | "LANG")
                || name.starts_with("LC_")
                || manifest.permissions.env.iter().any(|d| *d == name)
        })
        .collect()
}

fn resolve(dir: &Path, command: &str) -> PathBuf {
    dir.join(command)
}

type Pipe = Transport<BufReader<std::process::ChildStdout>, std::process::ChildStdin>;

fn converse<S: Storage>(
    storage: &mut S,
    transport: &mut Pipe,
    plugin: &Plugin,
    binding: &RunBinding,
    grant: &Grant,
    session: &mut Session,
) -> Result<std::result::Result<RunFinish, &'static str>> {
    let init = InitializeParams {
        protocol: PROTOCOL_VERSION.into(),
        manifest_id: plugin.manifest.id.clone(),
        run: binding.clone(),
    };
    if transport
        .send(&Envelope::request(1, INITIALIZE, init)?)
        .is_err()
    {
        return Ok(Err("interrupted"));
    }
    let reply = match transport.read() {
        Ok(Some(reply)) => reply,
        Ok(None) => return Ok(Err("interrupted")),
        Err(_) => return Ok(Err("protocol")),
    };
    let Ok(init) = reply.result::<InitializeResult>() else {
        return Ok(Err("protocol"));
    };
    if major(&init.protocol) != major(PROTOCOL_VERSION) || init.manifest_id != plugin.manifest.id {
        return Ok(Err("protocol"));
    }
    loop {
        let message = match transport.read() {
            Ok(Some(message)) => message,
            Ok(None) => return Ok(Err("interrupted")),
            Err(_) => return Ok(Err("protocol")),
        };
        match message.method.as_deref() {
            Some(EVIDENCE_SUBMIT) => {
                session.counts.submitted += 1;
                let checked = message
                    .params()
                    .map_err(|error| format!("{error:#}"))
                    .and_then(|submit| validate(submit, grant, Utc::now()));
                match checked {
                    Ok(evidence) => {
                        storage.append_evidence(binding.run_id, &evidence)?;
                        session.counts.accepted += 1;
                    }
                    Err(why) => {
                        eprintln!("rejected evidence: {}", display_safe(&why));
                        session.counts.rejected += 1;
                    }
                }
            }
            Some(COVERAGE_REPORT) => {
                let Ok(report) = message.params::<CoverageReport>() else {
                    return Ok(Err("protocol"));
                };
                let bound = report.source.as_ref().is_none_or(|s| *s == binding.source)
                    && grant.allows_window(&report.window)
                    && report.scope.as_ref().is_none_or(|s| *s == binding.scope);
                if bound {
                    session.coverage = Some(report);
                } else {
                    eprintln!("rejected coverage report: not the bound source, scope or window");
                }
            }
            Some(RUN_FINISH) => {
                let Ok(finish) = message.params::<RunFinish>() else {
                    return Ok(Err("protocol"));
                };
                let reply = Envelope::response(message.id, session.counts)?;
                let _ = transport.send(&reply);
                return Ok(Ok(finish));
            }
            _ => return Ok(Err("protocol")),
        }
    }
}
