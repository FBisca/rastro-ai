//! The host's teardown against real processes: it kills the connector's whole process group and
//! ends its helper threads, whatever state the leader is in. The connector is launched by the
//! host's own `run`, so these tests drive it directly through the pieces that run leaves hidden.

use rastro_core::host::{Connector, Limits};
use std::process::Command;
use std::time::{Duration, Instant};

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
            max_claims: None,
            extraction_time: Duration::from_secs(5),
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
        max_claims: None,
        extraction_time: Duration::from_secs(5),
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
