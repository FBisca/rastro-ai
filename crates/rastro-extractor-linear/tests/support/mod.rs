//! The scripted Linear API and a home with the real Linear connector and extractor installed,
//! driving the real `rastro` binary.
#![allow(dead_code)]
#[path = "../../../rastro-connector-linear/tests/support/api.rs"]
pub mod api;
#[path = "../../../rastro-cli/tests/support/binaries.rs"]
mod binaries;
#[path = "../../../rastro-connector-linear/tests/support/home.rs"]
pub mod home;
pub use api::*;
pub use home::Env;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// A home with the Linear connector and the Linear extractor installed, and source `l`.
pub fn env(api: &str) -> Env {
    Env::new(
        built("rastro"),
        api,
        &[
            home::Install {
                id: "linear",
                manifest: include_str!("../../../rastro-connector-linear/rastro-plugin.toml"),
                command: built("rastro-connector-linear"),
                vocabulary: None,
            },
            home::Install {
                id: "rastro-extractor-linear",
                manifest: include_str!("../../rastro-plugin.toml"),
                command: extractor_binary(),
                vocabulary: Some(include_str!("../../vocabulary.toml")),
            },
        ],
    )
}

/// The extractor under test, as a private copy no concurrent build can rewrite.
pub fn extractor_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro-extractor-linear")))
    })
}
/// `rastro` and the Linear connector live in other packages: build them next to the extractor
/// once and copy each.
fn built(name: &'static str) -> &'static Path {
    static BUILT: OnceLock<()> = OnceLock::new();
    static RASTRO: OnceLock<PathBuf> = OnceLock::new();
    static CONNECTOR: OnceLock<PathBuf> = OnceLock::new();
    let extractor = Path::new(env!("CARGO_BIN_EXE_rastro-extractor-linear"));
    BUILT.get_or_init(|| {
        let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        build.args([
            "build",
            "--quiet",
            "-p",
            "rastro-cli",
            "-p",
            "rastro-connector-linear",
        ]);
        if extractor.parent().is_some_and(|d| d.ends_with("release")) {
            build.arg("--release");
        }
        assert!(build.status().unwrap().success(), "building rastro failed");
    });
    let slot = if name == "rastro" {
        &RASTRO
    } else {
        &CONNECTOR
    };
    slot.get_or_init(|| binaries::private_copy(&extractor.with_file_name(name)))
}
