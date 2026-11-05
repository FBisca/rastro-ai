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

/// The sorted names of an object schema's properties.
fn property_names(schema: &serde_json::Value) -> Vec<&str> {
    // An optional object is a union of the object and null.
    let object = ["anyOf", "oneOf"]
        .iter()
        .find_map(|key| schema[key].as_array())
        .and_then(|variants| variants.iter().find(|v| v.get("properties").is_some()))
        .unwrap_or(schema);
    let mut names: Vec<_> = object["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    names.sort();
    names
}

/// S13: about and why are additive v2, `sync.extraction` and `claims.rebuild.pairs` are recorded
/// additions, and existing schemas are unchanged
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
        let mut new = expand(new, &generated["$defs"]);
        // Additive v2 changes recorded in docs/decisions/surfaces.md, each with the fields it
        // may carry: anything else that differs, or any other field, is a breaking change.
        match old["name"].as_str().unwrap() {
            "sync" => {
                let run = &mut new["result"]["properties"]["runs"]["items"];
                let extraction = run["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("extraction");
                assert_eq!(
                    property_names(&extraction.unwrap()),
                    [
                        "counts",
                        "evidence",
                        "extractor",
                        "extractor_version",
                        "reason",
                        "run_id",
                        "state"
                    ]
                );
                assert!(
                    !run["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("extraction"))
                );
            }
            "claims.rebuild" => {
                let result = new["result"].as_object_mut().unwrap();
                let pairs = result["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("pairs");
                assert_eq!(
                    property_names(&pairs.unwrap()["items"]),
                    [
                        "counts",
                        "extractor",
                        "extractor_version",
                        "reason",
                        "run_id",
                        "scope",
                        "source",
                        "state"
                    ]
                );
                result["required"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|name| name != "pairs");
            }
            _ => {}
        }
        assert_eq!(new, old.clone());
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
