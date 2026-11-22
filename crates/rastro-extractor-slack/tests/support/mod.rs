//! The scripted Slack API (and the scripted Linear API for cross-source tests), and a home
//! with the real Slack connector and extractor installed, driving the real `rastro` binary.
#![allow(dead_code)]
#[path = "../../../rastro-connector-slack/tests/support/api.rs"]
pub mod api;
#[path = "../../../rastro-cli/tests/support/binaries.rs"]
mod binaries;
pub mod home;
#[path = "../../../rastro-connector-linear/tests/support/api.rs"]
pub mod linear_api;
pub use api::*;
pub use home::Env;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// A home with the Slack connector and the Slack extractor installed, and source `s`.
pub fn env(api: &str) -> Env {
    Env::new(built("rastro"), api, &slack_installs())
}

/// A home that also installs the Linear connector and extractor, for cross-source tests.
pub fn env_with_linear(api: &str) -> Env {
    let mut plugins = slack_installs();
    plugins.extend([
        home::Install {
            id: "linear",
            manifest: include_str!("../../../rastro-connector-linear/rastro-plugin.toml"),
            command: built("rastro-connector-linear"),
            vocabulary: None,
        },
        home::Install {
            id: "rastro-extractor-linear",
            manifest: include_str!("../../../rastro-extractor-linear/rastro-plugin.toml"),
            command: built("rastro-extractor-linear"),
            vocabulary: Some(include_str!(
                "../../../rastro-extractor-linear/vocabulary.toml"
            )),
        },
    ]);
    Env::new(built("rastro"), api, &plugins)
}

fn slack_installs() -> Vec<home::Install<'static>> {
    vec![
        home::Install {
            id: "slack",
            manifest: include_str!("../../../rastro-connector-slack/rastro-plugin.toml"),
            command: built("rastro-connector-slack"),
            vocabulary: None,
        },
        home::Install {
            id: "rastro-extractor-slack",
            manifest: include_str!("../../rastro-plugin.toml"),
            command: extractor_binary(),
            vocabulary: Some(include_str!("../../vocabulary.toml")),
        },
    ]
}

/// The extractor under test, as a private copy no concurrent build can rewrite.
pub fn extractor_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        binaries::private_copy(Path::new(env!("CARGO_BIN_EXE_rastro-extractor-slack")))
    })
}

/// `rastro` and the connectors live in other packages: build them next to the extractor once
/// and copy each. The Linear pair is built only when a cross-source test asks for it.
fn built(name: &'static str) -> &'static Path {
    static BUILT: OnceLock<()> = OnceLock::new();
    static LINEAR_BUILT: OnceLock<()> = OnceLock::new();
    static RASTRO: OnceLock<PathBuf> = OnceLock::new();
    static SLACK: OnceLock<PathBuf> = OnceLock::new();
    static LINEAR_CONNECTOR: OnceLock<PathBuf> = OnceLock::new();
    static LINEAR_EXTRACTOR: OnceLock<PathBuf> = OnceLock::new();
    let extractor = Path::new(env!("CARGO_BIN_EXE_rastro-extractor-slack"));
    let release = extractor.parent().is_some_and(|d| d.ends_with("release"));
    let build = |packages: &[&str]| {
        let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        build.args(["build", "--quiet"]);
        for package in packages {
            build.args(["-p", package]);
        }
        if release {
            build.arg("--release");
        }
        assert!(
            build.status().unwrap().success(),
            "building {packages:?} failed"
        );
    };
    let slot = match name {
        "rastro" => {
            BUILT.get_or_init(|| build(&["rastro-cli", "rastro-connector-slack"]));
            &RASTRO
        }
        "rastro-connector-slack" => {
            BUILT.get_or_init(|| build(&["rastro-cli", "rastro-connector-slack"]));
            &SLACK
        }
        "rastro-connector-linear" => {
            LINEAR_BUILT.get_or_init(|| {
                build(&["rastro-connector-linear", "rastro-extractor-linear"]);
            });
            &LINEAR_CONNECTOR
        }
        _ => {
            LINEAR_BUILT.get_or_init(|| {
                build(&["rastro-connector-linear", "rastro-extractor-linear"]);
            });
            &LINEAR_EXTRACTOR
        }
    };
    slot.get_or_init(|| binaries::private_copy(&extractor.with_file_name(name)))
}
