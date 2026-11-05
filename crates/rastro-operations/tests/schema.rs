#![allow(non_snake_case)]
//! The committed v2 schema is the catalog's output, byte for byte.

/// surfaces/R1: the generator's output equals the committed file; any difference fails
#[test]
fn generated_schema_equals_the_committed_file() {
    let generated = rastro_operations::schema::generate();
    let committed = include_str!("../../../schema/operations-v2.schema.json");
    assert!(
        generated == committed,
        "schema/operations-v2.schema.json drifted from the catalog; run \
         `cargo run -p rastro-operations --bin gen-schema` and commit the result"
    );
}

/// surfaces/R1: about and why are additive v2 and existing schemas are unchanged
#[test]
fn about_and_why_are_additive_v2_and_existing_schemas_are_unchanged() {
    use serde_json::{Value, json};
    let generated: Value = serde_json::from_str(&rastro_operations::schema::generate()).unwrap();
    let existing: Value =
        serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json")).unwrap();
    assert_eq!(generated["properties"]["schema_version"]["const"], 2);
    let ops = generated["x-operations"].as_array().unwrap();
    for name in ["about", "why"] {
        let op = ops.iter().find(|op| op["name"] == name).unwrap();
        assert_eq!(op["surfaces"], json!(["cli", "mcp"]));
        assert!(op["input"]["properties"].get("limit").is_some());
    }
    // Resolve referenced definitions: unchanged roots cannot hide changed shared types.
    fn expand(value: &Value, defs: &Value) -> Value {
        match value {
            Value::Object(map) if map.contains_key("$ref") => expand(
                &defs[map["$ref"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("#/$defs/")
                    .unwrap()],
                defs,
            ),
            Value::Object(map) => map
                .iter()
                .map(|(key, value)| (key.clone(), expand(value, defs)))
                .collect(),
            Value::Array(items) => items.iter().map(|value| expand(value, defs)).collect(),
            other => other.clone(),
        }
    }
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/existing-operations.json")).unwrap();
    for old in baseline.as_array().unwrap() {
        let new = ops.iter().find(|op| op["name"] == old["name"]).unwrap();
        assert_eq!(expand(new, &generated["$defs"]), old.clone());
    }
    assert!(
        generated == existing,
        "committed schema differs from generated catalog"
    );
}

/// surfaces/R30: catalog declares claim cli read paths without changing existing routes
#[test]
fn catalog_declares_claim_cli_read_paths_without_changing_existing_routes() {
    use rastro_operations::catalog::CATALOG;
    for spec in &CATALOG {
        assert_eq!(spec.read_path, ["about", "why"].contains(&spec.name));
        if spec.read_path {
            assert!(spec.read_only);
        }
    }
}
