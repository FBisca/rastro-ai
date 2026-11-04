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
                    help.contains(&format!(
                        "[default: {}]",
                        default
                            .as_str()
                            .map_or_else(|| default.to_string(), String::from)
                    )),
                    "{name}: default of {} missing in {help}",
                    input.name
                );
            }
        }
        // options: exactly the catalog's, besides `--json`, `--help` and `--version`
        let mut listed: Vec<_> = help
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("--"))
            .map(|l| l.split([' ', ',']).next().unwrap().to_string())
            .filter(|f| !["json", "help", "version"].contains(&f.as_str()))
            .collect();
        let mut expected: Vec<_> = inputs
            .iter()
            .filter(|i| !i.positional)
            .map(|i| i.name.replace('_', "-"))
            .collect();
        listed.sort();
        expected.sort();
        assert_eq!(listed, expected, "{name}: {help}");
        // positionals: exactly the catalog's
        let mut shown: Vec<_> = help
            .lines()
            .skip_while(|l| *l != "Arguments:")
            .skip(1)
            .take_while(|l| !l.is_empty())
            .map(|l| {
                l.split_whitespace()
                    .next()
                    .unwrap()
                    .trim_matches(['<', '>', '[', ']'])
                    .to_lowercase()
            })
            .collect();
        let mut positional: Vec<_> = inputs
            .iter()
            .filter(|i| i.positional)
            .map(|i| i.name.clone())
            .collect();
        shown.sort();
        positional.sort();
        assert_eq!(shown, positional, "{name}: {help}");
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
    // the schema is strict: extra keys, missing keys and wrong values all fail
    let bad = |value: Value, why: &str| {
        assert!(
            validator.validate(&value).is_err(),
            "accepted: {why}: {value}"
        );
    };
    let envelope = |operation: &str, result: Value| serde_json::json!({"schema_version": 2, "operation": operation, "result": result});
    bad(
        envelope(
            "index.rebuild",
            serde_json::json!({"indexed": 0, "junk": 1}),
        ),
        "extra key",
    );
    bad(
        envelope("index.rebuild", serde_json::json!({})),
        "missing key",
    );
    bad(
        envelope(
            "coverage",
            serde_json::json!({"coverage": [{"source": "a", "scope": "w", "state": "found", "junk": 1}]}),
        ),
        "extra key in coverage",
    );
    let run = |status: &str| {
        serde_json::json!({"run_id": 1, "source": "a", "scope": "w", "plugin_id": "p",
        "plugin_version": "1", "status": status, "reason": null,
        "started_at": "2026-11-01T00:00:00Z", "finished_at": "2026-11-01T00:00:00Z",
        "window": {"from": "2026-11-01T00:00:00Z", "to": "2026-11-01T00:00:00Z"},
        "counts": {"submitted": 0, "accepted": 0, "rejected": 0, "unchanged": 0},
        "coverage": "found"})
    };
    let good = envelope("sync", serde_json::json!({"runs": [run("ok")]}));
    validator.validate(&good).unwrap();
    bad(
        envelope("sync", serde_json::json!({"runs": [run("exploded")]})),
        "bad status",
    );
    let window = |state: &str| {
        serde_json::json!({"from": "2026-11-01T00:00:00Z", "to": "2026-11-01T00:00:00Z",
        "state": state, "selection": "all", "checked_at": "2026-11-01T00:00:00Z"})
    };
    let covering = |w: Value| {
        envelope(
            "coverage",
            serde_json::json!({"coverage": [{"source": "a", "scope": "w", "registered": true, "state": "found", "windows_total": 1, "windows": [w]}]}),
        )
    };
    validator.validate(&covering(window("found"))).unwrap();
    bad(
        covering(window("unknown")),
        "window state outside its three",
    );
    bad(
        envelope(
            "source.list",
            serde_json::json!({"sources": [{"name": "", "connector": "github", "selector": {}, "scope": "w", "freshness_rule": "1h", "window_start": "2026-11-01T00:00:00Z", "keep_original": false}]}),
        ),
        "empty name",
    );
    bad(
        envelope(
            "source.list",
            serde_json::json!({"sources": [{"name": "a", "connector": "github", "selector": {}, "scope": "w", "freshness_rule": "soon", "window_start": "2026-11-01T00:00:00Z", "keep_original": false}]}),
        ),
        "bad freshness rule",
    );
    // v1 stays exactly as it was
    let v1 = include_bytes!("../../../schema/operations-v1.schema.json");
    let hash = v1.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    });
    assert_eq!(hash, V1_FNV, "schema/operations-v1.schema.json changed");
}
const V1_FNV: u64 = 0x6976af6cc700be4;
