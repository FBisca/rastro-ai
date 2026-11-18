#![allow(non_snake_case)]
mod support;

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

/// Set on each child the concurrency test starts: the binary being republished, and the file whose
/// appearance ends that child's run.
const CHILD_SOURCE: &str = "RASTRO_SNAPSHOT_SOURCE";
const CHILD_STOP: &str = "RASTRO_SNAPSHOT_STOP";
/// How many test processes snapshot the same binary at once, and how often it is republished.
const CHILDREN: usize = 6;
const PUBLICATIONS: u32 = 3;
/// The window a publication leaves the binary missing, and how long a published build holds still.
const ABSENT: Duration = Duration::from_millis(25);
const STABLE: Duration = Duration::from_millis(75);
/// A child stops when the stop file appears; this bounds it if that file never comes.
const MAX_ROUNDS: usize = 200;

/// spec: A test runs its own binary snapshot
/// A snapshot of a binary is the build that was published when the test started.
#[test]
fn snapshot_is_the_current_build() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tool");
    let write = |bytes: &[u8], at: std::time::SystemTime| {
        fs::write(&source, bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(at)
            .unwrap();
    };
    let at = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    write(b"old build", at);
    let old = support::binaries::private_copy(&source);
    assert_eq!(fs::read(&old).unwrap(), b"old build");
    // A different build of the same length and the same modification time.
    write(b"new build", at);
    let new = support::binaries::private_copy(&source);
    assert_eq!(fs::read(&new).unwrap(), b"new build");
    assert_ne!(old, new);
    // The earlier snapshot is untouched.
    assert_eq!(fs::read(&old).unwrap(), b"old build");
}

/// spec: A test runs its own binary snapshot
///
/// Many processes, each in its own snapshot, while another publishes builds of the same binary the
/// way a `cargo` does: it removes the old build first, so the binary a test is copying is gone for a
/// moment. Every test runs its own snapshot to the end of its run, and none of them fails on a
/// snapshot a sibling swept or on a binary that was briefly missing.
#[test]
fn many_processes_snapshot_and_sweep_while_the_build_is_republished() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("tool");
    let stop = dir.path().join("stop");
    publish(&source, 0);
    let children: Vec<_> = (0..CHILDREN)
        .map(|_| {
            Command::new(env::current_exe().unwrap())
                .args(["--exact", "a_snapshot_of_a_published_binary_runs"])
                .env(CHILD_SOURCE, &source)
                .env(CHILD_STOP, &stop)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for build in 1..=PUBLICATIONS {
        fs::remove_file(&source).unwrap();
        thread::sleep(ABSENT);
        publish(&source, build);
        thread::sleep(STABLE);
    }
    fs::write(&stop, b"").unwrap();
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "a test process failed while it snapshotted the same binary:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// spec: A test runs its own binary snapshot
///
/// Alone it proves a snapshot of a just-published binary is a runnable copy of it. The concurrency
/// test starts it once per child process, and each child then snapshots and runs the republished
/// binary in a loop, sweeping the directory every time as a test process does.
#[test]
fn a_snapshot_of_a_published_binary_runs() {
    let (Ok(source), Ok(stop)) = (env::var(CHILD_SOURCE), env::var(CHILD_STOP)) else {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("tool");
        publish(&source, 1);
        assert_whole_build(&run(&support::binaries::private_copy(&source)));
        return;
    };
    let (source, stop) = (PathBuf::from(source), PathBuf::from(stop));
    let mut first = None;
    for _ in 0..MAX_ROUNDS {
        let snapshot = support::binaries::private_copy(&source);
        assert_whole_build(&run(&snapshot));
        first.get_or_insert(snapshot);
        if stop.exists() {
            break;
        }
    }
    // Every sibling swept the directory while this process ran, and the first snapshot is still
    // here and still runs: no test takes away what another is running.
    assert_whole_build(&run(first.as_ref().unwrap()));
}

/// Writes build `build` of a script that prints its own number, as `cargo` publishes a binary.
fn publish(source: &Path, build: u32) {
    fs::write(source, format!("#!/bin/sh\nprintf 'build-{build}\\n'\n")).unwrap();
    fs::set_permissions(source, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs a snapshot and returns what it printed: a snapshot that was swept, or that is not
/// executable, fails here.
fn run(snapshot: &Path) -> String {
    let out = Command::new(snapshot)
        .output()
        .unwrap_or_else(|error| panic!("running {}: {error}", snapshot.display()));
    assert!(
        out.status.success(),
        "{} exited {:?}: {}",
        snapshot.display(),
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The printed build is whole: a copy taken while the build was being written prints a cut line.
fn assert_whole_build(printed: &str) {
    let build = printed
        .strip_prefix("build-")
        .and_then(|rest| rest.strip_suffix('\n'));
    assert!(
        build.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
        "a snapshot of a partly written build ran: {printed:?}"
    );
}
