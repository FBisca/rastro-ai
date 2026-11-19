//! A fake action provider: a shell script that speaks the plugin protocol and answers every
//! action message with a canned success, so no outside system is ever called. It writes each
//! message it receives to `calls.log` beside itself, so a test can tell that none reached it.
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

/// The id of the fake provider's manifest.
pub const ID: &str = "fake-actions";

const SCRIPT: &str = r#"#!/bin/sh
log="$(dirname "$0")/calls.log"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocol":"1.0","manifest_id":"fake-actions"}}\n' "$id"
      ;;
    *'"method":"action.lookup"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"found":false}}\n' "$id"
      ;;
    *'"method":"action.execute"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"found":true,"identifier":"FAKE-1","url":"https://fake.invalid/FAKE-1"}}\n' "$id"
      ;;
    *'"method":"action.undo"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"found":false}}\n' "$id"
      ;;
  esac
done
"#;

/// A valid manifest declaring `linear.create_issue`, with no evidence permission.
pub fn manifest() -> toml::Table {
    r#"id = "fake-actions"
name = "Rastro fake action provider"
version = "0.1.0"
author = "Rastro"
kinds = ["action_provider"]
protocol = 1
command = "provider"

[permissions]
submit = []
scopes = []
env = []
actions = ["linear.create_issue"]

[[action]]
name = "linear.create_issue"
reach = "others_see_it"
reversibility = "can_be_undone"
"#
    .parse()
    .unwrap()
}

/// Installs the fake provider under `<config>/plugins/fake-actions` with `edit` applied to its
/// manifest.
pub fn install(config: &Path, edit: impl FnOnce(&mut toml::Table)) {
    let dir = config.join("plugins").join(ID);
    fs::create_dir_all(&dir).unwrap();
    let mut manifest = manifest();
    edit(&mut manifest);
    fs::write(dir.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
    let script = dir.join("provider");
    fs::write(&script, SCRIPT).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Every message the fake provider has received, one per line; empty when it was never started.
pub fn calls(config: &Path) -> String {
    fs::read_to_string(config.join("plugins").join(ID).join("calls.log")).unwrap_or_default()
}
