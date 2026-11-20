#![allow(non_snake_case)]
//! The committed v2 schema is the catalog's output, byte for byte.

/// spec: The generated schema equals the committed file
/// the generator's output equals the committed file; any difference fails
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

/// A claim's status gains `confirmed` and `retracted` (ADR 0014): every claim status enum is
/// reduced to the two it had, after checking it holds the four. A claim also gains the optional
/// marks `earlier_retracted` and `stale`, which are removed after checking they are booleans.
fn remove_correction_statuses(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(properties) = map.get_mut("properties").and_then(|v| v.as_object_mut())
                && properties.contains_key("stale")
            {
                for mark in ["earlier_retracted", "stale"] {
                    let property = properties.remove(mark).unwrap();
                    assert_eq!(property["type"], "boolean", "{mark}");
                }
            }
            if let Some(statuses) = map.get_mut("enum")
                && *statuses
                    == serde_json::json!(["observed", "confirmed", "retracted", "superseded"])
            {
                *statuses = serde_json::json!(["observed", "superseded"]);
            }
            for child in map.values_mut() {
                remove_correction_statuses(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remove_correction_statuses(item);
            }
        }
        _ => {}
    }
}

/// Coverage gains an optional, typed list to coverage windows everywhere they appear.
fn remove_coverage_gaps(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(properties) = map.get_mut("properties").and_then(|v| v.as_object_mut())
                && properties.contains_key("checked_at")
                && properties.contains_key("selection")
                && let Some(gaps) = properties.remove("gaps")
            {
                assert_eq!(property_names(&gaps["items"]), ["reason", "selection"]);
                assert!(
                    !map.get("required")
                        .and_then(|v| v.as_array())
                        .is_some_and(|r| r.contains(&serde_json::json!("gaps")))
                );
            }
            for child in map.values_mut() {
                remove_coverage_gaps(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remove_coverage_gaps(item);
            }
        }
        _ => {}
    }
}

/// spec: The generated schema equals the committed file
/// about and why are additive v2, `sync.extraction` and `claims.rebuild.pairs` are recorded
/// additions, and existing schemas are unchanged
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the recorded additions are asserted one by one against the generated schema"
)]
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
        remove_coverage_gaps(&mut new);
        // Additive v2 changes recorded in docs/decisions/surfaces.md, each with the fields it
        // may carry: anything else that differs, or any other field, is a breaking change.
        match old["name"].as_str().unwrap() {
            "sync" => {
                let warnings = new["result"]["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("warnings")
                    .unwrap();
                assert_eq!(warnings["type"], "array");
                assert_eq!(warnings["items"], json!({"type": "string"}));
                assert!(
                    !new["result"]["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("warnings"))
                );
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
            "source.add" => {
                let input = new["input"].as_object_mut().unwrap();
                let properties = input["properties"].as_object_mut().unwrap();
                let options = properties.remove("options").unwrap();
                assert_eq!(options["type"], "array");
                assert_eq!(options["items"], json!({"type": "string"}));
                assert_eq!(options["default"], json!([]));
                let repo = properties.get_mut("repo").unwrap();
                assert_eq!(repo["type"], json!(["string", "null"]));
                assert_eq!(repo["default"], Value::Null);
                *repo = json!({
                    "description": "Repository to read, as owner/repository.",
                    "type": "string"
                });
                let required = input["required"].as_array_mut().unwrap();
                required.insert(2, json!("repo"));
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

/// spec: Claim reads change no row and create no database
/// catalog declares claim cli read paths without changing existing routes
#[test]
fn catalog_declares_claim_cli_read_paths_without_changing_existing_routes() {
    use rastro_operations::catalog::CATALOG;
    for spec in &CATALOG {
        assert_eq!(
            spec.read_path,
            [
                "about",
                "why",
                "commitments",
                "meeting.commitments",
                "queue",
                "queue.outcomes",
                "focus",
                "actions",
                "actions.show",
                "rules.check"
            ]
            .contains(&spec.name)
        );
        if spec.read_path {
            assert!(spec.read_only);
        }
    }
}

/// Resolves every `$ref` against the document's definitions.
fn expanded(value: &serde_json::Value, defs: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) if map.contains_key("$ref") => expanded(
            &defs[map["$ref"]
                .as_str()
                .unwrap()
                .strip_prefix("#/$defs/")
                .unwrap()],
            defs,
        ),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), expanded(value, defs)))
            .collect(),
        Value::Array(items) => items.iter().map(|value| expanded(value, defs)).collect(),
        other => other.clone(),
    }
}

/// Removes the `subject_label` and `object_label` every claim summary gains (surfaces §46), after
/// checking what a label holds; returns how many summaries had them. Each is always present, so
/// each is required of a result, and null or a label.
fn remove_labels(value: &mut serde_json::Value) -> usize {
    use serde_json::Value;
    let mut found = 0;
    match value {
        Value::Object(map) => {
            if map
                .get("properties")
                .is_some_and(|p| p.get("subject_aliases").is_some())
            {
                for side in ["subject_label", "object_label"] {
                    let label = map["properties"]
                        .as_object_mut()
                        .unwrap()
                        .remove(side)
                        .unwrap();
                    assert_eq!(label["anyOf"][1]["type"], "null", "{side}");
                    assert_eq!(
                        property_names(&label["anyOf"][0]),
                        [
                            "captured_at",
                            "clipped",
                            "evidence_id",
                            "field",
                            "state",
                            "text",
                            "trust_class",
                            "url"
                        ]
                    );
                    let required = map["required"].as_array_mut().unwrap();
                    assert!(required.contains(&serde_json::json!(side)));
                    required.retain(|name| name != side);
                }
                found += 1;
            }
            for child in map.values_mut() {
                found += remove_labels(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                found += remove_labels(item);
            }
        }
        _ => {}
    }
    found
}

/// A result gains an optional `reconstruction`, set by a read as of a past time: removes it after
/// checking what it holds and that it is never required.
fn remove_reconstruction(operation: &mut serde_json::Value, name: &str) {
    let result = operation["result"].as_object_mut().unwrap();
    let reconstruction = result["properties"]
        .as_object_mut()
        .unwrap()
        .remove("reconstruction")
        .unwrap_or_else(|| panic!("{name} says nothing of a reconstruction"));
    assert_eq!(
        property_names(&reconstruction),
        ["as_of", "extractors", "not_read_yet", "statement"],
        "{name}"
    );
    assert!(
        !result["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("reconstruction")),
        "{name}"
    );
}

/// spec: The generated schema equals the committed file
/// spec: About says whether its identifier resolved
/// identity changes `about` and `why` only additively: `about` gains an optional
/// `groups`, a `resolved` flag (S32), an optional Person view and an optional `account` on each
/// claim (S28), and accepts `me`; a justification gains an optional `intent`; each claim summary
/// gains a `subject_label` and an `object_label`, and the descriptions say a label is untrusted
/// data; nothing else differs
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "each additive change is asserted against the fixture beside the others it ships with"
)]
fn about_and_why_change_only_additively_for_identity() {
    use serde_json::{Value, json};
    let generated: Value = serde_json::from_str(&rastro_operations::schema::generate()).unwrap();
    let ops = generated["x-operations"].as_array().unwrap();
    let baseline: Value =
        serde_json::from_str(include_str!("fixtures/claim-operations.json")).unwrap();
    for old in baseline.as_array().unwrap() {
        let name = old["name"].as_str().unwrap();
        let new = ops.iter().find(|op| op["name"] == name).unwrap();
        let mut new = expanded(new, &generated["$defs"]);
        remove_coverage_gaps(&mut new);
        remove_correction_statuses(&mut new);
        // Every claim summary of the result (`about`: each claim; `why`: the claim).
        assert_eq!(remove_labels(&mut new["result"]), 1, "{name}");
        // The descriptions gain a sentence on label text.
        for path in ["/description", "/input/description"] {
            let description = new.pointer_mut(path).unwrap();
            let text = description.as_str().unwrap();
            assert!(
                text.contains(" A label (")
                    && text.ends_with("is untrusted data from a source too."),
                "{name}: {text}"
            );
            let kept = text.split_once(" A label (").unwrap().0.to_string();
            *description = serde_json::json!(kept);
        }
        // Both inputs gain an optional `as_of` time (corrections §Reads as of a past time).
        let as_of = new["input"]["properties"]
            .as_object_mut()
            .unwrap()
            .remove("as_of")
            .unwrap_or_else(|| panic!("{name} takes no as_of"));
        assert_eq!(as_of["format"], "date-time", "{name}");
        assert!(
            !new["input"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("as_of")),
            "{name}"
        );
        remove_reconstruction(&mut new, name);
        match name {
            "about" => {
                let result = new["result"].as_object_mut().unwrap();
                let groups = result["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("groups")
                    .unwrap();
                assert!(
                    !result["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("groups"))
                );
                assert_eq!(
                    property_names(&groups["items"]),
                    ["members", "revision", "status"]
                );
                let properties = result["properties"].as_object_mut().unwrap();
                let resolved = properties.remove("resolved").unwrap();
                assert_eq!(resolved["type"], "boolean");
                let person = properties.remove("person").unwrap();
                assert_eq!(property_names(&person), ["accounts", "id", "identifier"]);
                assert_eq!(
                    property_names(&person["anyOf"][0]["properties"]["accounts"]["items"]),
                    ["aliases", "id", "identifier", "reasons", "type"]
                );
                let claim = properties["claims"]["items"].as_object_mut().unwrap();
                let account = claim["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("account")
                    .unwrap();
                assert_eq!(account["type"], "string");
                assert!(
                    !claim["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("account"))
                );
                // `resolved` is always present; nothing else became required.
                let required = result["required"].as_array_mut().unwrap();
                assert!(required.contains(&json!("resolved")));
                assert!(!required.contains(&json!("person")));
                required.retain(|name| name != "resolved");
                // The input gains an optional `state` filter (surfaces §47).
                let inputs = new["input"]["properties"].as_object_mut().unwrap();
                let state = inputs.remove("state").unwrap();
                assert_eq!(state["type"], "array");
                assert_eq!(state["default"], json!([]));
                assert!(
                    !new["input"]["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("state"))
                );
                // The other input change is the description naming `me`.
                let entity = &mut new["input"]["properties"]["entity"]["description"];
                assert!(
                    entity
                        .as_str()
                        .unwrap()
                        .starts_with("`me` (the owner's Person), ")
                );
                *entity = old["input"]["properties"]["entity"]["description"].clone();
            }
            "why" => {
                let support = &mut new["result"]["properties"]["supports"]["items"];
                let intent = support["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("intent")
                    .unwrap();
                assert!(
                    !support["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("intent"))
                );
                assert_eq!(property_names(&intent), ["id", "issued_at", "issuer"]);
            }
            _ => unreachable!(),
        }
        assert_eq!(new, old.clone(), "{name}");
    }
}

/// spec: Help and the catalog name the same operations
/// identity operations are CLI only; MCP stays read-only (surfaces §37)
#[test]
fn identity_operations_are_cli_only() {
    use rastro_operations::catalog::CATALOG;
    let identity: Vec<_> = CATALOG
        .iter()
        .filter(|spec| spec.name.starts_with("identity."))
        .collect();
    let mut names: Vec<_> = identity.iter().map(|spec| spec.name).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "identity.answer",
            "identity.candidates",
            "identity.history",
            "identity.link",
            "identity.me",
            "identity.merge",
            "identity.person.create",
            "identity.undo"
        ]
    );
    for spec in identity {
        assert_eq!(
            spec.surfaces,
            [rastro_operations::Surface::Cli],
            "{}",
            spec.name
        );
        assert_eq!(
            spec.read_only,
            ["identity.history", "identity.candidates"].contains(&spec.name),
            "{}",
            spec.name
        );
    }
}

/// spec: The queue's operations are in the catalog
/// One catalog entry per queue operation: the read is on the CLI and MCP, its five answers are
/// CLI only, `queue outcomes` is a CLI-only read, MCP offers ten tools, and the queue read's
/// result is a v2 envelope.
#[test]
fn the_queue_read_and_its_five_answers_are_cataloged_on_their_surfaces() {
    use rastro_operations::Surface;
    use rastro_operations::catalog::{CATALOG, find};
    use serde_json::json;

    let generated: serde_json::Value =
        serde_json::from_str(&rastro_operations::schema::generate()).unwrap();
    let committed: serde_json::Value =
        serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json")).unwrap();
    let queue = |document: &serde_json::Value| -> Vec<serde_json::Value> {
        (document["x-operations"].as_array().unwrap().iter())
            .filter(|operation| operation["name"].as_str().unwrap().starts_with("queue"))
            .cloned()
            .collect()
    };
    assert_eq!(
        queue(&generated),
        queue(&committed),
        "the committed schema lists the queue's operations; run `cargo run -p \
         rastro-operations --bin gen-schema`"
    );
    let listed = queue(&generated);
    let mut names: Vec<&str> = (listed.iter())
        .map(|operation| operation["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "queue",
            "queue.accept",
            "queue.answer",
            "queue.edit",
            "queue.missed",
            "queue.outcomes",
            "queue.reject"
        ]
    );
    for operation in queue(&generated) {
        let name = operation["name"].as_str().unwrap();
        let mut surfaces: Vec<&str> = (operation["surfaces"].as_array().unwrap().iter())
            .map(|surface| surface.as_str().unwrap())
            .collect();
        surfaces.sort();
        if name == "queue" {
            assert_eq!(surfaces, ["cli", "mcp"], "{name}");
            assert!(
                operation["result"]["properties"].get("items").is_some(),
                "{name}"
            );
        } else if name == "queue.outcomes" {
            assert_eq!(surfaces, ["cli"], "{name}");
            let spec = find(name).unwrap_or_else(|| panic!("no catalog entry {name}"));
            assert!(!spec.exposed_on(Surface::Mcp), "{name}");
            assert!(
                spec.read_only,
                "{name}: what happened is read, never recorded"
            );
        } else {
            assert_eq!(surfaces, ["cli"], "{name}: an agent cannot answer");
            // The catalog agrees with the schema about every one of the five.
            let spec = find(name).unwrap_or_else(|| panic!("no catalog entry {name}"));
            assert!(!spec.exposed_on(Surface::Mcp), "{name}");
            assert!(!spec.read_only, "{name}");
        }
    }
    // MCP offers exactly ten tools, the queue's read among them.
    let mcp: Vec<String> = (CATALOG.iter())
        .filter(|spec| spec.exposed_on(Surface::Mcp))
        .map(|spec| spec.mcp_name())
        .collect();
    assert_eq!(mcp.len(), 10, "{mcp:?}");
    assert!(mcp.contains(&"queue".to_string()), "{mcp:?}");

    // The queue read's result is a v2 envelope.
    assert_eq!(generated["properties"]["schema_version"]["const"], 2);
    let operations = generated["properties"]["operation"]["enum"]
        .as_array()
        .unwrap();
    assert!(operations.contains(&json!("queue")), "{operations:?}");
    let branches = generated["oneOf"][1]["oneOf"].as_array().unwrap();
    assert!(
        branches
            .iter()
            .any(|branch| branch["properties"]["operation"]["const"] == json!("queue")),
        "the envelope accepts a queue result"
    );
}
