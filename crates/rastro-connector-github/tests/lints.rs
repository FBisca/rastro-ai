//! The plugin crates inherit the workspace's size and complexity lints.

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

/// The plugin crates: the GitHub and Linear connectors and extractors, and the test extractor.
const PLUGINS: [&str; 5] = [
    "rastro-connector-github",
    "rastro-connector-linear",
    "rastro-extractor-github",
    "rastro-extractor-linear",
    "rastro-test-extractor",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest(krate: &str) -> Value {
    let path = workspace_root()
        .join("crates")
        .join(krate)
        .join("Cargo.toml");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

// spec: Every crate inherits the workspace lints
#[test]
fn the_plugin_crates_inherit_the_workspace_lints() {
    for krate in PLUGINS {
        let inherits = manifest(krate)
            .get("lints")
            .and_then(|lints| lints.get("workspace"))
            .and_then(Value::as_bool);
        assert_eq!(
            inherits,
            Some(true),
            "{krate} does not inherit the workspace lints"
        );
    }
}
