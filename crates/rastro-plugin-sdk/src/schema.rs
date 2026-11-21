//! Generates `schema/plugin-protocol-v1.schema.json` from the connector role's messages and the
//! plugin manifest. The root schema describes one JSON-RPC line of a connector sync; the manifest
//! lives at `#/$defs/Manifest` for an author to validate the JSON form of `rastro-plugin.toml`.
use crate::{
    Counts, CoverageReport, EvidenceSubmit, InitializeParams, InitializeResult, Manifest, RunFinish,
};
use schemars::schema_for;
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

/// Splits one type's root schema into the schema of its own type and its definitions.
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
                "two messages define `{name}` differently"
            );
        }
    }
    object.remove("$schema");
    root
}

/// The JSON-RPC id: a string or a number, as the envelope's well-formedness check allows.
fn id() -> Value {
    json!({"type": ["string", "number"]})
}

/// A request (`with_id`) or notification carrying `method` and `params`.
fn call(method: &str, params: Value, with_id: bool) -> Value {
    let mut properties = Map::new();
    properties.insert("jsonrpc".into(), json!({"const": "2.0"}));
    let mut required = vec![json!("jsonrpc")];
    if with_id {
        properties.insert("id".into(), id());
        required.push(json!("id"));
    }
    properties.insert("method".into(), json!({"const": method}));
    required.push(json!("method"));
    properties.insert("params".into(), params);
    required.push(json!("params"));
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

/// A response carrying `result`.
fn reply(result: Value) -> Value {
    json!({
        "type": "object",
        "properties": {
            "jsonrpc": {"const": "2.0"},
            "id": id(),
            "result": result,
        },
        "required": ["jsonrpc", "id", "result"],
        "additionalProperties": false,
    })
}

pub fn generate() -> String {
    let mut definitions = Map::new();
    let initialize_params = absorb(schema_for!(InitializeParams).to_value(), &mut definitions);
    let initialize_result = absorb(schema_for!(InitializeResult).to_value(), &mut definitions);
    let evidence_submit = absorb(schema_for!(EvidenceSubmit).to_value(), &mut definitions);
    let coverage_report = absorb(schema_for!(CoverageReport).to_value(), &mut definitions);
    let run_finish = absorb(schema_for!(RunFinish).to_value(), &mut definitions);
    let counts = absorb(schema_for!(Counts).to_value(), &mut definitions);
    let manifest = absorb(schema_for!(Manifest).to_value(), &mut definitions);
    definitions.insert("Manifest".into(), manifest);
    let branches = vec![
        call("initialize", initialize_params, true),
        reply(initialize_result),
        call("evidence.submit", evidence_submit, false),
        call("coverage.report", coverage_report, false),
        call("run.finish", run_finish, true),
        reply(counts),
    ];
    let document = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://rastro.dev/schema/plugin-protocol-v1.schema.json",
        "title": "Rastro plugin protocol v1",
        "description": "One line of the connector protocol a sync exchanges, and the plugin \
            manifest at #/$defs/Manifest. Generated from rastro-plugin-sdk; do not edit.",
        "oneOf": branches,
        "$defs": definitions,
    });
    let mut text = serde_json::to_string_pretty(&canonical(document)).expect("schema serialises");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Manifest;

    // spec: The generated plugin protocol schema equals the committed file
    /// The generator's output equals the committed file; any difference fails.
    #[test]
    fn generated_schema_equals_the_committed_file() {
        let generated = generate();
        let committed = include_str!("../../../schema/plugin-protocol-v1.schema.json");
        assert!(
            generated == committed,
            "schema/plugin-protocol-v1.schema.json drifted from the connector messages and the \
             manifest; run `cargo run -p rastro-plugin-sdk --bin gen-schema` and commit the result"
        );
    }

    /// A connector manifest with a declared source option.
    const CONNECTOR: &str = r#"
id = "work"
name = "Work mail"
version = "0.1.0"
author = "An author"
kinds = ["connector"]
protocol = 1
command = "work-connector"

[permissions]
submit = ["issue", "comment"]
scopes = ["personal"]
actions = []
env = ["WORK_TOKEN"]

[[source_options]]
name = "mailbox"
required = true
pattern = ".+"
help = "The mailbox to read"
"#;

    /// An action provider manifest: its actions ride the wire name `action`.
    const ACTION_PROVIDER: &str = r#"
id = "linear"
name = "Linear actions"
version = "0.1.0"
author = "An author"
kinds = ["action_provider"]
protocol = 1
command = "linear-actions"

[permissions]
submit = []
scopes = []
actions = ["create_issue"]

[[action]]
name = "create_issue"
reach = "others_see_it"
reversibility = "can_be_undone"
"#;

    // spec: The generated plugin protocol schema equals the committed file
    /// The manifest schema accepts what the manifest parser accepts, and publishes the actions
    /// field under its wire name `action`.
    #[test]
    fn the_manifest_schema_accepts_what_the_parser_accepts() {
        let document: Value = serde_json::from_str(&generate()).unwrap();
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "#/$defs/Manifest",
            "$defs": document["$defs"],
        });
        let validator = jsonschema::validator_for(&schema).unwrap();
        for text in [CONNECTOR, ACTION_PROVIDER] {
            let manifest = Manifest::parse(text).unwrap();
            let value = serde_json::to_value(&manifest).unwrap();
            validator
                .validate(&value)
                .unwrap_or_else(|error| panic!("{error}: {value}"));
        }
        let properties = document["$defs"]["Manifest"]["properties"]
            .as_object()
            .unwrap();
        assert!(
            properties.contains_key("action"),
            "the manifest's actions are published as `action`"
        );
        assert!(!properties.contains_key("actions"));
    }
}
