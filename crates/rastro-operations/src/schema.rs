//! Generates `schema/operations-v2.schema.json` from the catalog.
use crate::{
    catalog::{CATALOG, Surface},
    envelope::SCHEMA_VERSION,
};
use serde_json::{Map, Value, json};

/// Rebuilds objects with sorted keys so the file does not depend on map ordering features.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// Splits one operation's root schema into the schema of its own type and its definitions.
fn absorb(root: Value, definitions: &mut Map<String, Value>) -> Value {
    let mut root = root;
    let object = root.as_object_mut().expect("object schema");
    if let Some(Value::Object(defs)) = object.remove("$defs") {
        for (name, definition) in defs {
            let kept = definitions
                .entry(name.clone())
                .or_insert_with(|| definition.clone());
            assert_eq!(
                *kept, definition,
                "two operations define `{name}` differently"
            );
        }
    }
    object.remove("$schema");
    root
}

pub fn generate() -> String {
    let mut definitions = Map::new();
    let mut operations = Vec::new();
    let mut branches = Vec::new();
    for spec in &CATALOG {
        let input = absorb(spec.input_schema(), &mut definitions);
        let output = absorb(spec.output_schema(), &mut definitions);
        let surfaces: Vec<_> = spec.surfaces.iter().map(|s| s.as_str()).collect();
        let mut operation = json!({
            "name": spec.name,
            "description": spec.description(),
            "surfaces": surfaces,
            "input": input,
            "result": output,
        });
        // Only an operation whose MCP tool name differs from its name says so.
        if spec.exposed_on(Surface::Mcp) && spec.mcp_name() != spec.name {
            operation["mcp_tool"] = json!(spec.mcp_name());
        }
        operations.push(operation);
        branches.push(json!({
            "properties": {
                "operation": {"const": spec.name},
                "result": output,
            }
        }));
    }
    let names: Vec<_> = CATALOG.iter().map(|s| s.name).collect();
    let document = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://rastro.dev/schema/operations-v2.schema.json",
        "title": "Rastro operations v2",
        "description": "Every operation's input and result, and the envelope `--json` prints. Generated from the operation catalog; do not edit.",
        "type": "object",
        "required": ["schema_version", "operation"],
        "properties": {
            "schema_version": {"const": SCHEMA_VERSION},
            "operation": {"enum": names},
            "error": {"type": "string", "minLength": 1},
            "result": {"type": "object"},
        },
        "additionalProperties": false,
        "oneOf": [
            {"required": ["error"], "not": {"required": ["result"]}},
            {
                "required": ["result"],
                "not": {"required": ["error"]},
                "oneOf": branches,
            },
        ],
        "x-operations": operations,
        "$defs": definitions,
    });
    let mut text = serde_json::to_string_pretty(&canonical(document)).expect("schema serialises");
    text.push('\n');
    text
}
