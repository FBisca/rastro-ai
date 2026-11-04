//! Shared by the end-to-end suites: the binaries they drive.
#![allow(dead_code)]
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

pub mod binaries;

/// The `rastro` binary under test, as a private copy.
pub fn rastro() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro"))))
}

/// The scripted test connector, built next to `rastro` and copied.
pub fn test_connector() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", "rastro-test-connector"])
            .status()
            .unwrap();
        assert!(status.success(), "building the test connector failed");
        let original =
            Path::new(env!("CARGO_BIN_EXE_rastro")).with_file_name("rastro-test-connector");
        binaries::private_copy(&original)
    })
}

/// The scripted test extractor, built next to `rastro` and copied.
pub fn test_extractor() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--quiet", "-p", "rastro-test-extractor"])
            .status()
            .unwrap();
        assert!(status.success(), "building the test extractor failed");
        let original =
            Path::new(env!("CARGO_BIN_EXE_rastro")).with_file_name("rastro-test-extractor");
        binaries::private_copy(&original)
    })
}
