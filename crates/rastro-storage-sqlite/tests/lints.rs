//! The workspace's size and complexity lints, read at this crate's own manifest.

use std::fs;

use toml::Value;

// spec: Every crate inherits the workspace lints
#[test]
fn the_storage_crate_inherits_the_workspace_lints() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let manifest: Value =
        toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
    let inherits = manifest
        .get("lints")
        .and_then(|lints| lints.get("workspace"))
        .and_then(Value::as_bool);
    assert_eq!(
        inherits,
        Some(true),
        "rastro-storage-sqlite does not inherit the workspace lints"
    );
}
