#![allow(non_snake_case)]
use rastro_core::{ReadStorage, Refusal};
use rastro_storage_sqlite::SqliteStorage;
use std::{fs, os::unix::fs::PermissionsExt};

/// surfaces/S19: a store that can't be read without writing is a typed refusal
#[test]
fn S19_an_unreadable_store_is_a_typed_refusal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rastro.sqlite3");
    drop(SqliteStorage::open(&path).unwrap());
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(directory.path().join(format!("rastro.sqlite3{suffix}")));
    }
    let restore = || fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700));
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    // As root the directory stays writable and nothing is refused: nothing to pin.
    let writable = fs::File::create(directory.path().join("probe")).is_ok();
    let outcome = SqliteStorage::open_read_only(&path)
        .and_then(|store| store.expect("the file exists").read(|_| Ok(())));
    restore().unwrap();
    if writable {
        return;
    }
    let error = outcome.expect_err("a WAL database can't be read without its side files");
    assert!(
        matches!(
            error.downcast_ref::<Refusal>(),
            Some(Refusal::NotReadable(_))
        ),
        "{error:#}"
    );
}
