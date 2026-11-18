//! The workspace's size and complexity lints, read at the manifests of the crates that carry
//! offenders.

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

/// The crates that inherit the workspace lints while marking their offenders with `#[expect]`.
const CRATES: [&str; 2] = ["rastro-core", "rastro-operations"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// spec: Every crate inherits the workspace lints
#[test]
fn a_crate_with_offenders_inherits_the_workspace_lints() {
    for krate in CRATES {
        let path = workspace_root()
            .join("crates")
            .join(krate)
            .join("Cargo.toml");
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
            "{krate} does not inherit the workspace lints"
        );
    }
}
