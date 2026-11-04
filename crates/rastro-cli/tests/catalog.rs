#![allow(non_snake_case)]
//! The CLI is generated from the operation catalog; its envelopes follow the v2 schema.
mod support;
use rastro_operations::{Surface, catalog};
use serde_json::Value;
use std::process::Command;

fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(support::rastro())
        .args(args)
        .env("RASTRO_HOME", home)
        .output()
        .unwrap()
}

/// S2: `--help` lists exactly the inputs, defaults and descriptions the catalog defines
#[test]
fn S2_help_lists_exactly_what_the_catalog_defines() {
    let home = tempfile::tempdir().unwrap();
    for name in ["search", "coverage"] {
        let spec = catalog::find(name).unwrap();
        assert!(spec.surfaces.contains(&Surface::Cli));
        let out = run(home.path(), &[name, "--help"]);
        let help = String::from_utf8(out.stdout).unwrap();
        // clap drops the final full stop of a one-line help text
        let shown = |text: &str| text.trim_end_matches('.').to_string();
        assert!(help.contains(&shown(&spec.description())), "{name}: {help}");
        let inputs = spec.inputs();
        for input in &inputs {
            assert!(
                help.contains(&shown(&input.description)),
                "{name}: {} missing in {help}",
                input.description
            );
            let flag = format!("--{}", input.name.replace('_', "-"));
            let shown = if input.positional {
                input.name.to_uppercase()
            } else {
                flag
            };
            assert!(help.contains(&shown), "{name}: {shown} missing in {help}");
            if let Some(default) = &input.default
                && default != &Value::Bool(false)
                && !default.is_null()
            {
                assert!(
                    help.contains(&format!("[default: {}]", default.as_str().unwrap_or(""))),
                    "{name}: default of {} missing in {help}",
                    input.name
                );
            }
        }
        // no option beyond the catalog's, `--json`, `--help` and `--version`
        let listed: Vec<_> = help
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("--"))
            .map(|l| l.split([' ', ',']).next().unwrap().to_string())
            .filter(|f| !["json", "help", "version"].contains(&f.as_str()))
            .collect();
        let expected: Vec<_> = inputs
            .iter()
            .filter(|i| !i.positional)
            .map(|i| i.name.replace('_', "-"))
            .collect();
        assert_eq!(listed, expected, "{name}: {help}");
    }
}

/// S5: every command's `--json` envelope is version 2, valid against the v2 schema; v1 is frozen
#[test]
fn S5_every_envelope_is_version_2_and_schema_valid() {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let home = tempfile::tempdir().unwrap();
    let commands: &[&[&str]] = &[
        &[
            "source",
            "add",
            "demo",
            "--connector",
            "github",
            "--repo",
            "a/b",
            "--scope",
            "work",
        ],
        &["source", "list"],
        &["coverage"],
        &["coverage", "demo"],
        &["coverage", "missing"],
        &["sync", "demo"],
        &["search", "anything"],
        &["search", "anything", "--history"],
        &["index", "rebuild"],
        &["search"],
    ];
    for args in commands {
        let mut full = args.to_vec();
        full.push("--json");
        let out = run(home.path(), &full);
        let value: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {:?}", out));
        assert_eq!(value["schema_version"], 2, "{args:?}");
        validator
            .validate(&value)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {value}"));
    }
    // v1 stays exactly as it was
    let v1 = include_bytes!("../../../schema/operations-v1.schema.json");
    let hash = v1.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    });
    assert_eq!(hash, V1_FNV, "schema/operations-v1.schema.json changed");
}
const V1_FNV: u64 = 0x6976af6cc700be4;
