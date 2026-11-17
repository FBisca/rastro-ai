//! Private snapshots of the binaries a test runs.
//!
//! Every `cargo` invocation re-publishes `target/debug/<bin>`, and several test suites (or a
//! second `cargo test`) can do so while this one is executing it; macOS kills a binary whose
//! file changes under it (SIGKILL, empty stderr). Each test process therefore takes its own
//! snapshot, named for the process, and never reuses one by metadata: the bytes it runs are the
//! bytes the source held when it copied.
//!
//! A publish removes the old binary before it links the new one, so the source is missing for a
//! moment and a copy of it is torn while it is written. `private_copy` waits for a copy the source
//! held still for, and `sweep` takes away only what a process that no longer exists left behind.
#![allow(dead_code)]
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

/// How long a source may be missing or keep changing before `private_copy` gives up.
const PATIENCE: Duration = Duration::from_secs(10);
/// How long to let a source that was missing or changing settle before looking again.
const PAUSE: Duration = Duration::from_millis(10);

/// The size and modification time of a file, or `None` while it is not there: another `cargo`
/// removes a binary before it publishes a new build in its place.
fn stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos()))
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The process a file in the snapshot directory belongs to: `<binary>-<pid>-<nonce>` for a
/// snapshot, `<binary>.<pid>.partial` for a copy in progress. Anything else belongs to nobody here.
fn owner(name: &str) -> Option<u32> {
    let (_, pid) = match name.strip_suffix(".partial") {
        Some(partial) => partial.rsplit_once('.')?,
        None => name.rsplit_once('-')?.0.rsplit_once('-')?,
    };
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    pid.parse().ok()
}

/// Removes what test processes that no longer exist left behind: their snapshots and the partial
/// copies of a run that died mid-copy. A file of a process that is still running stays, whatever it
/// is: from here it looks exactly like the copy a live test is making right now.
fn sweep(directory: &Path) {
    let entries: Vec<fs::DirEntry> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    let gone: BTreeSet<u32> = entries
        .iter()
        .filter_map(|entry| owner(&entry.file_name().to_string_lossy()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|pid| !alive(*pid))
        .collect();
    for entry in entries {
        if owner(&entry.file_name().to_string_lossy()).is_some_and(|pid| gone.contains(&pid)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A fresh copy of `source` for this process, made while `source` held still. Call it once per
/// binary (behind a `OnceLock`); the copy lives as long as the process.
pub fn private_copy(source: &Path) -> PathBuf {
    let directory = source.parent().unwrap().join("rastro-test-binaries");
    fs::create_dir_all(&directory).unwrap();
    sweep(&directory);
    let name = source.file_name().unwrap().to_string_lossy().into_owned();
    let nonce = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let partial = directory.join(format!("{name}.{pid}.partial"));
    // Only this process writes its partial copy; an earlier one left it behind by dying mid-copy.
    let _ = fs::remove_file(&partial);
    let deadline = Instant::now() + PATIENCE;
    loop {
        // Skipped while the source is missing, which is the first moment of every publish.
        if let Some(before) = stamp(source) {
            let copied = fs::copy(source, &partial);
            if copied.is_ok_and(|size| size == before.0) && stamp(source) == Some(before) {
                let target = directory.join(format!("{name}-{pid}-{nonce}"));
                fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).unwrap();
                fs::rename(&partial, &target).unwrap();
                return target;
            }
        }
        let _ = fs::remove_file(&partial);
        assert!(
            Instant::now() < deadline,
            "{} was missing or kept changing for {}s and was never copied whole",
            source.display(),
            PATIENCE.as_secs()
        );
        thread::sleep(PAUSE);
    }
}
