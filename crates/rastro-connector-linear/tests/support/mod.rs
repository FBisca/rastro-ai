//! The scripted Linear API and a home with the Linear connector installed.
#![allow(dead_code)]
pub mod api;
#[path = "../../../rastro-cli/tests/support/binaries.rs"]
mod binaries;
pub mod home;
pub use api::*;
pub use home::Env;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// A home with only the connector installed (its extractor is bound but not installed).
pub fn env(api: &str) -> Env {
    Env::new(
        rastro_binary(),
        api,
        &[home::Install {
            id: "linear",
            manifest: include_str!("../../rastro-plugin.toml"),
            command: connector_binary(),
            vocabulary: None,
        }],
    )
}

pub fn env_with_extractor(api: &str) -> Env {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    let executable = BINARY.get_or_init(|| {
        let connector = Path::new(env!("CARGO_BIN_EXE_rastro-connector-linear"));
        let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        build.args(["build", "--quiet", "-p", "rastro-extractor-linear"]);
        if connector.parent().is_some_and(|d| d.ends_with("release")) {
            build.arg("--release");
        }
        assert!(build.status().unwrap().success());
        binaries::private_copy(&connector.with_file_name("rastro-extractor-linear"))
    });
    Env::new(
        rastro_binary(),
        api,
        &[
            home::Install {
                id: "linear",
                manifest: include_str!("../../rastro-plugin.toml"),
                command: connector_binary(),
                vocabulary: None,
            },
            home::Install {
                id: "rastro-extractor-linear",
                manifest: include_str!("../../../rastro-extractor-linear/rastro-plugin.toml"),
                command: executable,
                vocabulary: Some(include_str!(
                    "../../../rastro-extractor-linear/vocabulary.toml"
                )),
            },
        ],
    )
}

/// The connector under test, as a private copy no concurrent build can rewrite.
pub fn connector_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro-connector-linear")))
    })
}
/// `rastro` lives in another package, so build it next to the connector once and copy it.
fn rastro_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let connector = Path::new(env!("CARGO_BIN_EXE_rastro-connector-linear"));
        let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        build.args(["build", "--quiet", "-p", "rastro-cli"]);
        if connector.parent().is_some_and(|d| d.ends_with("release")) {
            build.arg("--release");
        }
        assert!(build.status().unwrap().success(), "building rastro failed");
        binaries::private_copy(&connector.with_file_name("rastro"))
    })
}
