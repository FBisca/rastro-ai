//! The Slack app manifest template the connector ships for the owner's own app.
#![allow(non_snake_case)]

/// spec: The Slack app template asks only for the read scopes
#[test]
fn the_app_manifest_template_asks_only_for_the_read_scopes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/slack-app-manifest.yaml");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("shipped template: {e}"));
    let manifest: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
    let user = manifest["oauth_config"]["scopes"]["user"]
        .as_sequence()
        .expect("the template declares user scopes");
    let names: Vec<&str> = user.iter().map(|s| s.as_str().unwrap()).collect();
    // The five read scopes, as the spec names them.
    assert_eq!(
        names,
        [
            "channels:read",
            "channels:history",
            "groups:read",
            "groups:history",
            "users:read"
        ]
    );
    assert!(
        manifest["oauth_config"]["scopes"]["bot"].is_null(),
        "the template declares bot scopes: {manifest:?}"
    );
    assert_eq!(
        manifest["settings"]["token_rotation_enabled"].as_bool(),
        Some(false),
        "token rotation must be off: {manifest:?}"
    );
}
