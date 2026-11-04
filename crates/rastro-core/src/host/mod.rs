//! The plugin host: finds a connector, spawns it, and ingests what it submits (ADR 0003).
//! Sequential and happy-path first; hardening against crashes and silence builds on `drive`.
use crate::evidence::{Grant, validate};
use crate::runs::RunReport;
use crate::{Appended, Counts, CoverageRecord, CoverageState, Locations, RunEnd, Source, Storage};
use anyhow::{Result, bail};
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
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant},
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
    let run_id = storage.start_run(
        source,
        (&plugin_id, &plugin_version),
        started_at,
        (from, to),
    )?;
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
        Err(_) => Some("storage".into()),
        Ok(Ok(finish)) if finish.status != "ok" => Some(finish.reason.as_deref().map_or_else(
            || reported.clone().unwrap_or_else(|| "protocol".into()),
            |r| typed(Some(r)),
        )),
        Ok(Ok(_)) if reported.is_some() => reported,
        Ok(Ok(_)) if coverage.is_none() => Some("protocol".into()),
        Ok(Ok(_)) => None,
    };
    let status = if reason.is_some() { "failed" } else { "ok" };
    // The connector reports completeness; the core decides found or empty.
    let (state, coverage_reason, retry_at, selection) = match coverage {
        Some(report) if reason.is_none() => {
            let state = if counts.accepted + counts.unchanged > 0 {
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
    /// How long a connector may take to exit after `run.finish`.
    shutdown: Duration,
    /// Stderr lines relayed per run.
    stderr_lines: usize,
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
            shutdown: Duration::from_millis(number(
                "RASTRO_HOST_SHUTDOWN_MS",
                DEFAULT_SHUTDOWN.as_millis() as u64,
            )),
            stderr_lines: number("RASTRO_HOST_STDERR_LINES", DEFAULT_STDERR_LINES as u64) as usize,
        }
    }
}
/// The longest line a connector may send.
pub const DEFAULT_MAX_LINE: usize = 1 << 20;
/// How long a connector may stay silent before it is killed.
pub const DEFAULT_SILENCE: Duration = Duration::from_secs(60);

/// How long a connector may take to exit after `run.finish` before it is killed.
pub const DEFAULT_SHUTDOWN: Duration = Duration::from_secs(5);
/// How many stderr lines are relayed per run; the rest are counted and dropped.
pub const DEFAULT_STDERR_LINES: usize = 1000;

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
    let mut command = Command::new(command);
    command
        .env_clear()
        .envs(allowed_environment(&plugin.manifest));
    let Ok(mut connector) = Connector::launch(command, &plugin.manifest.id, Limits::from_env())
    else {
        return Ok(Err("not_installed"));
    };
    let outcome = match converse(
        storage,
        &mut connector.wire,
        &connector.relay,
        plugin,
        binding,
        grant,
        session,
    ) {
        Err(error) => {
            connector.relay.line(&format!("storage error: {error:#}"));
            Ok(Err("storage"))
        }
        outcome => outcome,
    };
    connector.teardown(matches!(outcome, Ok(Ok(_))));
    outcome
}

/// A running connector and everything the host holds to talk to it.
struct Connector {
    child: Child,
    wire: Wire,
    relay: io::Relay,
    limits: Limits,
    tally: io::Tally,
}
impl Connector {
    /// Starts the command with piped stdio and its own process group,
    /// so everything it starts can be stopped with it.
    fn launch(mut command: Command, id: &str, limits: Limits) -> std::io::Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command.spawn()?;
        let (stdin, stdout, stderr) = (
            child.stdin.take().expect("piped stdin"),
            child.stdout.take().expect("piped stdout"),
            child.stderr.take().expect("piped stderr"),
        );
        let tally = io::Tally::default();
        let relay = io::start_relay(io::Printer::global(), id.into(), limits.stderr_lines);
        io::drain_stderr(stderr, relay.clone(), &tally);
        let wire = Wire {
            jobs: io::write_lines(stdin, &tally),
            events: io::read_lines(stdout, limits.max_line, &tally),
            deadline: limits.silence,
        };
        Ok(Self {
            child,
            wire,
            relay,
            limits,
            tally,
        })
    }

    /// Ends the connector. After a clean finish it gets the shutdown deadline to exit on its
    /// own; then its whole process group is killed, so a descendant can't keep the pipes open
    /// and the helper threads end. Returns the helper-thread tally for tests.
    fn teardown(self, finished: bool) -> io::Tally {
        self.teardown_observed(finished, |_| {})
    }

    /// `before_signal` runs with the leader's pid just before the group is signalled; tests use
    /// it to see the leader is still unreaped at that moment.
    fn teardown_observed(mut self, finished: bool, before_signal: impl FnOnce(u32)) -> io::Tally {
        // Closing the job queue closes stdin, which is how a well-behaved connector exits.
        drop(self.wire);
        let pid = self.child.id();
        let deadline = Instant::now() + self.limits.shutdown;
        // Watch for the exit without reaping: the pid stays ours until the group is signalled.
        while finished && !has_exited(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        before_signal(pid);
        kill_group(pid);
        let _ = self.child.wait();
        drop(self.relay);
        // The helper threads end once the group is dead and the pipes are closed.
        let ended = Instant::now() + self.limits.shutdown;
        while self.tally.live() > 0 && Instant::now() < ended {
            std::thread::sleep(Duration::from_millis(5));
        }
        io::Printer::global().flush(self.limits.shutdown);
        self.tally
    }
}

/// Whether the child has exited, seen without reaping it (`waitid` with `WNOWAIT`).
fn has_exited(pid: u32) -> bool {
    #[cfg(unix)]
    {
        #[repr(C, align(8))]
        struct Info([u8; 128]);
        unsafe extern "C" {
            fn waitid(kind: i32, id: u32, info: *mut Info, options: i32) -> i32;
        }
        const P_PID: i32 = 1;
        const WEXITED: i32 = 4;
        const WNOHANG: i32 = 1;
        #[cfg(target_os = "macos")]
        const WNOWAIT: i32 = 0x20;
        #[cfg(not(target_os = "macos"))]
        const WNOWAIT: i32 = 0x0100_0000;
        let mut info = Info([0; 128]);
        // SAFETY: `info` is a zeroed, 8-aligned buffer at least as large as `siginfo_t`;
        // with WNOHANG the call returns at once and sets `si_signo` (the first four bytes)
        // only when the child has exited.
        let result = unsafe { waitid(P_PID, pid, &mut info, WEXITED | WNOWAIT | WNOHANG) };
        result != 0 || info.0[..4] != [0; 4]
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// SIGKILL to the process group led by `leader`, which `launch` created.
fn kill_group(leader: u32) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            safe fn killpg(group: i32, signal: i32) -> i32;
        }
        const SIGKILL: i32 = 9;
        let _ = killpg(leader as i32, SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = leader;
}

/// The two ends of the conversation: lines out under a deadline, and bounded, timed lines in.
struct Wire {
    jobs: std::sync::mpsc::Sender<io::Job>,
    events: Receiver<io::Event>,
    /// How long a write may take, and how long the connector may stay silent.
    deadline: Duration,
}
impl Wire {
    fn send(&mut self, message: &Envelope) -> std::result::Result<(), &'static str> {
        let line = serde_json::to_string(message).map_err(|_| "protocol")?;
        let (ack, acked) = std::sync::mpsc::sync_channel(1);
        self.jobs.send((line, ack)).map_err(|_| "interrupted")?;
        match acked.recv_timeout(self.deadline) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => Err("interrupted"),
            Err(RecvTimeoutError::Timeout) => Err("timeout"),
        }
    }
    fn recv(&mut self) -> std::result::Result<Envelope, &'static str> {
        match self.events.recv_timeout(self.deadline) {
            Ok(io::Event::Line(line)) => match serde_json::from_str::<Envelope>(&line) {
                Ok(envelope) if envelope.is_well_formed() => Ok(envelope),
                _ => Err("protocol"),
            },
            Ok(io::Event::TooLong | io::Event::Invalid) => Err("protocol"),
            Ok(io::Event::Eof) | Err(RecvTimeoutError::Disconnected) => Err("interrupted"),
            Err(RecvTimeoutError::Timeout) => Err("timeout"),
        }
    }
}

fn converse<S: Storage>(
    storage: &mut S,
    wire: &mut Wire,
    relay: &io::Relay,
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
    if reply.id != Some(1.into()) {
        return Ok(Err("protocol"));
    }
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
                    Ok(evidence) => match storage.append_evidence(binding.run_id, &evidence)? {
                        Appended::New(_) => session.counts.accepted += 1,
                        Appended::Unchanged(_) => session.counts.unchanged += 1,
                    },
                    Err(why) => {
                        relay.line(&format!("rejected evidence: {why}"));
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
                    relay.line("rejected coverage report: not the bound source, scope or window");
                }
            }
            Some(RUN_FINISH) => {
                if message.id.is_none() {
                    return Ok(Err("protocol"));
                }
                let Ok(finish) = message.params::<RunFinish>() else {
                    return Ok(Err("protocol"));
                };
                let reply = Envelope::response(message.id, session.counts)?;
                step!(wire.send(&reply));
                return Ok(Ok(finish));
            }
            _ => return Ok(Err("protocol")),
        }
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// evidence/R26: teardown kills the process group and releases every helper thread
    #[test]
    fn teardown_kills_the_process_group_and_releases_every_helper_thread() {
        for finished in [false, true] {
            let mut command = Command::new("/bin/sh");
            // A background child shares the pipes with the connector; neither will exit.
            command.args(["-c", "sleep 60 & exec sleep 60"]);
            let limits = Limits {
                max_line: 4096,
                silence: Duration::from_secs(1),
                shutdown: Duration::from_millis(200),
                stderr_lines: 10,
            };
            let connector = Connector::launch(command, "test", limits).unwrap();
            let tally = connector.teardown(finished);
            let started = Instant::now();
            while tally.live() > 0 {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "{} helper threads still hold the pipes",
                    tally.live()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    /// evidence/R27: the group is signalled while the exited leader is still unreaped
    #[test]
    fn the_group_is_signalled_while_the_exited_leader_is_still_unreaped() {
        let mut command = Command::new("/bin/sh");
        // The leader exits at once; its background child keeps the pipes.
        command.args(["-c", "sleep 60 & exit 0"]);
        let limits = Limits {
            max_line: 4096,
            silence: Duration::from_secs(1),
            shutdown: Duration::from_secs(5),
            stderr_lines: 10,
        };
        let connector = Connector::launch(command, "test", limits).unwrap();
        let mut state = String::new();
        connector.teardown_observed(true, |pid| {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            state = String::from_utf8_lossy(&output.stdout).trim().to_string();
        });
        assert!(
            state.starts_with('Z'),
            "leader was {state:?} when signalled, not a zombie"
        );
    }
}
