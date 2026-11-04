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
    COULDNT_LOOK_REASONS, COVERAGE_REPORT, CoverageReport, CoverageStatus, EVIDENCE_SUBMIT,
    Envelope, INITIALIZE, InitializeParams, InitializeResult, Manifest, PROTOCOL_VERSION,
    RUN_FINISH, RunBinding, RunFinish, Window, major,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::Duration,
};

mod io;

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
    let Session { counts, coverage } = session;
    let reported = coverage.as_ref().and_then(|c| {
        (c.status == CoverageStatus::CouldntLook).then(|| typed(c.reason.as_deref()))
    });
    // Why the run failed, if it did: what the host saw first, then what the connector said.
    let reason: Option<String> = match outcome {
        Ok(Err(reason)) => Some(reason.to_string()),
        Err(error) => {
            eprintln!("storage error: {}", display_safe(&format!("{error:#}")));
            Some("storage".into())
        }
        Ok(Ok(finish)) if finish.status != "ok" => Some(finish.reason.as_deref().map_or_else(
            || reported.clone().unwrap_or_else(|| "protocol".into()),
            |r| typed(Some(r)),
        )),
        Ok(Ok(_)) if reported.is_some() => reported,
        Ok(Ok(_)) => None,
    };
    let status = if reason.is_some() { "failed" } else { "ok" };
    // The connector reports completeness; the core decides found or empty.
    let (state, coverage_reason, retry_at, selection) = match coverage {
        Some(report) if reason.is_none() => {
            let state = if counts.accepted > 0 {
                CoverageState::Found
            } else {
                CoverageState::Empty
            };
            (state, None, None, report.selection)
        }
        Some(report) => (
            CoverageState::CouldNotLook,
            reason.clone(),
            report.retry_at.and_then(|t| t.parse().ok()),
            report.selection,
        ),
        // A connector that finishes without reporting coverage looked at nothing we can vouch for.
        None => (
            CoverageState::CouldNotLook,
            reason.clone().or(Some("protocol".into())),
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

/// What bounds a conversation. The core reads these from its own environment, which a
/// connector can neither see (it starts with a cleared one) nor change.
struct Limits {
    max_line: usize,
    silence: Duration,
}
impl Limits {
    fn from_env() -> Self {
        let number = |key: &str, default: u64| {
            env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        Self {
            max_line: number("RASTRO_HOST_MAX_LINE", DEFAULT_MAX_LINE as u64) as usize,
            silence: Duration::from_millis(number(
                "RASTRO_HOST_SILENCE_MS",
                DEFAULT_SILENCE.as_millis() as u64,
            )),
        }
    }
}
/// The longest line a connector may send.
pub const DEFAULT_MAX_LINE: usize = 1 << 20;
/// How long a connector may stay silent before it is killed.
pub const DEFAULT_SILENCE: Duration = Duration::from_secs(60);

/// A reason a connector gives must be one of the typed ones; anything else means it broke
/// the contract and is recorded as `protocol`.
fn typed(reason: Option<&str>) -> String {
    match reason {
        Some(r) if COULDNT_LOOK_REASONS.contains(&r) => r.into(),
        _ => "protocol".into(),
    }
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
        .stderr(Stdio::piped())
        .env_clear()
        .envs(allowed_environment(&plugin.manifest))
        .spawn()
    else {
        return Ok(Err("not_installed"));
    };
    let stdin = child.stdin.take().context("child stdin")?;
    let stdout = child.stdout.take().context("child stdout")?;
    let stderr = child.stderr.take().context("child stderr")?;
    let limits = Limits::from_env();
    let relay = io::relay_stderr(stderr, plugin.manifest.id.clone());
    let mut wire = Wire {
        stdin,
        events: io::read_lines(stdout, limits.max_line),
        silence: limits.silence,
    };
    let outcome = converse(storage, &mut wire, plugin, binding, grant, session);
    if !matches!(outcome, Ok(Ok(_))) {
        let _ = child.kill();
    }
    drop(wire.stdin);
    let _ = child.wait();
    let _ = relay.join();
    outcome
}

/// The two ends of the conversation: lines out, and bounded, timed lines in.
struct Wire {
    stdin: std::process::ChildStdin,
    events: Receiver<io::Event>,
    silence: Duration,
}
impl Wire {
    fn send(&mut self, message: &Envelope) -> std::result::Result<(), &'static str> {
        let line = serde_json::to_string(message).map_err(|_| "protocol")?;
        writeln!(self.stdin, "{line}")
            .and_then(|()| self.stdin.flush())
            .map_err(|_| "interrupted")
    }
    fn recv(&mut self) -> std::result::Result<Envelope, &'static str> {
        match self.events.recv_timeout(self.silence) {
            Ok(io::Event::Line(line)) => serde_json::from_str(&line).map_err(|_| "protocol"),
            Ok(io::Event::TooLong) => Err("protocol"),
            Ok(io::Event::Eof) | Err(RecvTimeoutError::Disconnected) => Err("interrupted"),
            Err(RecvTimeoutError::Timeout) => Err("timeout"),
        }
    }
}

fn converse<S: Storage>(
    storage: &mut S,
    wire: &mut Wire,
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
    macro_rules! step {
        ($e:expr) => {
            match $e {
                Ok(value) => value,
                Err(reason) => return Ok(Err(reason)),
            }
        };
    }
    step!(wire.send(&Envelope::request(1, INITIALIZE, init)?));
    let reply = step!(wire.recv());
    let Ok(init) = reply.result::<InitializeResult>() else {
        return Ok(Err("protocol"));
    };
    if major(&init.protocol) != major(PROTOCOL_VERSION) || init.manifest_id != plugin.manifest.id {
        return Ok(Err("protocol"));
    }
    loop {
        let message = step!(wire.recv());
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
                let _ = wire.send(&reply);
                return Ok(Ok(finish));
            }
            _ => return Ok(Err("protocol")),
        }
    }
}
