//! Private snapshots of the binaries a test runs.
//!
//! Every `cargo` invocation re-publishes `target/debug/<bin>`, and several test suites (or a
//! second `cargo test`) can do so while this one is executing it; macOS kills a binary whose
//! file changes under it (SIGKILL, empty stderr). Each test process therefore takes its own
//! snapshot, named for the process, and never reuses one by metadata: the bytes it runs are the
//! bytes the source held when it copied.
#![allow(dead_code)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::UNIX_EPOCH,
};

fn stamp(path: &Path) -> (u64, u128) {
    let meta = fs::metadata(path).unwrap();
    let modified = meta.modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
    (meta.len(), modified.as_nanos())
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Removes snapshots left by test processes that no longer exist.
fn sweep(directory: &Path) {
    for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `<binary>-<pid>-<nonce>`
        let mut parts = name.rsplitn(3, '-');
        let (_nonce, pid) = (parts.next(), parts.next());
        if let Some(pid) = pid.filter(|p| p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty())
            && !alive(pid)
        {
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
    for _ in 0..200 {
        let before = stamp(source);
        let partial = directory.join(format!("{name}.{pid}.partial"));
        let copied = fs::copy(source, &partial);
        if copied.is_ok_and(|n| n == before.0) && stamp(source) == before {
            let target = directory.join(format!("{name}-{pid}-{nonce}"));
            fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).unwrap();
            fs::rename(&partial, &target).unwrap();
            return target;
        }
        let _ = fs::remove_file(&partial);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("{} kept changing while it was copied", source.display());
}
