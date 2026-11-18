//! The workspace's size and complexity lints, inherited by this crate's manifest.

use std::fs;

use toml::Value;

fn manifest() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}

// spec: Every crate inherits the workspace lints
#[test]
fn rastro_cli_inherits_the_workspace_lints() {
    let inherits = manifest()
        .get("lints")
        .and_then(|lints| lints.get("workspace"))
        .and_then(Value::as_bool);
    assert_eq!(
        inherits,
        Some(true),
        "rastro-cli does not inherit the workspace lints"
    );
}
