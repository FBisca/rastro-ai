//! The authentication probe: the installed connector run once with `--check`, outside the plugin
//! protocol, so that no run, evidence, coverage or storage is involved.
use super::{Plugin, allowed_environment, resolve};
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// How long a connector gets to answer: past its own 30-second request timeout.
const PATIENCE: Duration = Duration::from_secs(45);
/// The most a connector's answer may weigh.
const LIMIT: u64 = 16 * 1024;

/// Runs the connector with `--check` under the environment a sync would give it and returns the
/// first line it printed on stdout. `None` when it could not start, said nothing, or ran past
/// its patience (it is killed). Its stderr is dropped, so nothing it says there reaches the owner.
pub(crate) fn probe(plugin: &Plugin) -> Option<String> {
    let mut child = Command::new(resolve(&plugin.dir, &plugin.manifest.command))
        .arg("--check")
        .env_clear()
        .envs(allowed_environment(&plugin.manifest))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.take(LIMIT).read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + PATIENCE;
    while child.try_wait().ok()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = reader.join().ok()?;
    text.lines().next().map(str::to_string)
}
