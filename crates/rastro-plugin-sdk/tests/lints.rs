//! The workspace's size and complexity lints, read at the manifests that declare and inherit them.

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

/// The lints every member inherits, each at clippy's own default threshold.
const LINTS: [&str; 5] = [
    "too_many_lines",
    "cognitive_complexity",
    "too_many_arguments",
    "fn_params_excessive_bools",
    "struct_excessive_bools",
];

/// The crates that inherit the workspace lints without carrying an offender.
const CLEAN: [&str; 3] = [
    "rastro-plugin-sdk",
    "rastro-adapter-claude-code",
    "rastro-test-connector",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

/// `value` at the nested `keys`, or `None` where any of them is absent.
fn lookup<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |value, key| value.get(*key))
}

// spec: Every crate inherits the workspace lints
#[test]
fn the_workspace_declares_every_size_and_complexity_lint() {
    let root = manifest(&workspace_root().join("Cargo.toml"));
    for lint in LINTS {
        let level = lookup(&root, &["workspace", "lints", "clippy", lint]).and_then(Value::as_str);
        assert_eq!(
            level,
            Some("warn"),
            "the workspace does not declare clippy::{lint}"
        );
    }
}

// spec: Every crate inherits the workspace lints
#[test]
fn a_crate_with_no_offender_inherits_the_workspace_lints() {
    for krate in CLEAN {
        let manifest = manifest(
            &workspace_root()
                .join("crates")
                .join(krate)
                .join("Cargo.toml"),
        );
        let inherits = lookup(&manifest, &["lints", "workspace"]).and_then(Value::as_bool);
        assert_eq!(
            inherits,
            Some(true),
            "{krate} does not inherit the workspace lints"
        );
    }
}
