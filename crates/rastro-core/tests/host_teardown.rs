//! The host's process handling against real processes: a connector starts with the environment it
//! is given and nothing else, and its teardown kills the whole process group and ends its helper
//! threads, whatever state the leader is in. The host's own `run` launches the connector, so these
//! tests drive it directly through the pieces that run leaves hidden.

use rastro_core::host::{Limits, launch_without_environment};
use std::process::Command;
use std::time::{Duration, Instant};

/// The deadlines these tests give a connector: short enough that a broken teardown fails them
/// within seconds, long enough that a healthy process finishes on its own.
fn limits(shutdown: Duration) -> Limits {
    Limits {
        max_line: 4096,
        silence: Duration::from_secs(1),
        shutdown,
        stderr_lines: 10,
        max_claims: None,
        extraction_time: Duration::from_secs(5),
    }
}

/// ADR 0004: the owner's environment never reaches a plugin. `HOME` is in every environment that
/// runs the suite, so a connector that reports one was handed the owner's environment. (`PATH` is
/// no probe: a shell invents its own when it has none.)
#[test]
fn a_connector_sees_none_of_the_owners_environment() {
    assert!(
        std::env::var_os("HOME").is_some(),
        "the test process needs HOME to tell an inherited environment from an empty one"
    );
    let home = tempfile::tempdir().unwrap();
    let seen = home.path().join("environment");
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        &format!("printf '%s' \"$HOME\" > '{}'", seen.display()),
    ]);
    let connector = launch_without_environment(command, limits(Duration::from_secs(5))).unwrap();
    connector.teardown(true);
    let reported = std::fs::read_to_string(&seen).unwrap();
    assert_eq!(
        reported, "",
        "the connector saw the owner's environment, not an empty one"
    );
}

/// evidence/R26: teardown kills the process group and releases every helper thread
#[test]
fn teardown_kills_the_process_group_and_releases_every_helper_thread() {
    for finished in [false, true] {
        let mut command = Command::new("/bin/sh");
        // A background child shares the pipes with the connector; neither will exit.
        command.args(["-c", "sleep 60 & exec sleep 60"]);
        let connector =
            launch_without_environment(command, limits(Duration::from_millis(200))).unwrap();
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
    let connector = launch_without_environment(command, limits(Duration::from_secs(5))).unwrap();
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
