//! The surfaces spec, read the way CI reads it: a spec states its requirements only as titled
//! `### Requirement:` blocks, and `scripts/spec_coverage.py` passes over the tree, so every title is
//! proven by a test tagged with it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The workspace root, from this crate's manifest.
fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The gate's own answer: what it printed on its error streams, and whether it accepted the tree.
fn run_gate(root: &Path) -> (bool, String) {
    let gate = root.join("scripts/spec_coverage.py");
    let output = Command::new("python3")
        .arg(&gate)
        .arg("--root")
        .arg(root)
        .output()
        .unwrap_or_else(|e| panic!("could not run {}: {e}", gate.display()));
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), printed)
}

/// spec: Specs hold only titled requirements
/// The surfaces spec's former `- **R<n>**` bullets are gone: it states its requirements only as
/// titled blocks, each with its scenarios, and the gate passes over the repository.
#[test]
fn the_surfaces_spec_holds_only_titled_requirements() {
    let root = workspace();
    let spec = std::fs::read_to_string(root.join("docs/specs/surfaces.md")).unwrap();

    let legacy: Vec<String> = spec
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("- **R") || line.trim() == "## Coverage")
        .map(|(number, line)| format!("{}: {line}", number + 1))
        .collect();
    assert!(
        legacy.is_empty(),
        "docs/specs/surfaces.md still holds legacy requirements: {legacy:?}"
    );
    assert!(
        spec.lines()
            .any(|line| line.starts_with("### Requirement: ")),
        "docs/specs/surfaces.md states no titled requirement"
    );
    let mut scenarios = spec
        .lines()
        .filter(|line| line.starts_with("#### Scenario: "));
    for title in spec
        .lines()
        .filter(|line| line.starts_with("### Requirement: "))
    {
        assert!(
            scenarios.next().is_some(),
            "docs/specs/surfaces.md: {title} states no scenario"
        );
    }

    let (passed, printed) = run_gate(&root);
    assert!(passed, "the spec gate refused the tree:\n{printed}");
}
