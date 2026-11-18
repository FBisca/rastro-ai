//! The workspace's specs and the tests that prove them, read by the repository's own spec gate.

use std::path::Path;
use std::process::Command;

/// The repository root, which holds `docs/specs/` and `scripts/`.
fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/rastro-cli sits two levels under the repository root")
}

// spec: Specs hold only titled requirements
#[test]
fn the_spec_gate_passes_over_the_workspace_specs() {
    let gate = root().join("scripts").join("spec_coverage.py");
    let output = Command::new("python3")
        .arg(&gate)
        .current_dir(root())
        .output()
        .unwrap_or_else(|e| panic!("running {}: {e}", gate.display()));
    assert!(
        output.status.success(),
        "the spec gate failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
