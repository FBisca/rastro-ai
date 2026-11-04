//! Private copies of the binaries a test runs.
//!
//! Every `cargo` invocation re-publishes `target/debug/<bin>`, and several test suites (or a
//! second `cargo test`) can do so while this one is executing it; macOS kills a binary whose
//! file changes under it (SIGKILL, empty stderr). A copy keyed by the source's size and
//! modification time is never rewritten, so nothing a test runs can change underneath it.
#![allow(dead_code)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

fn stamp(path: &Path) -> (u64, u128) {
    let meta = fs::metadata(path).unwrap();
    let modified = meta.modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
    (meta.len(), modified.as_nanos())
}

/// An immutable copy of `source`, made while `source` held still.
pub fn private_copy(source: &Path) -> PathBuf {
    let directory = source.parent().unwrap().join("rastro-test-binaries");
    fs::create_dir_all(&directory).unwrap();
    for _ in 0..200 {
        let before = stamp(source);
        let name = source.file_name().unwrap().to_string_lossy();
        let target = directory.join(format!("{name}-{}-{}", before.0, before.1));
        if target.exists() {
            return target;
        }
        let partial = directory.join(format!("{name}.{}.partial", std::process::id()));
        let copied = fs::copy(source, &partial);
        if copied.is_ok_and(|n| n == before.0) && stamp(source) == before {
            fs::set_permissions(&partial, fs::Permissions::from_mode(0o755)).unwrap();
            fs::rename(&partial, &target).unwrap();
            return target;
        }
        let _ = fs::remove_file(&partial);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("{} kept changing while it was copied", source.display());
}
