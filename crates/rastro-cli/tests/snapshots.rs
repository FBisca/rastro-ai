#![allow(non_snake_case)]
mod support;

use std::fs;

/// evidence/R45: a snapshot of a binary is the current build
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
