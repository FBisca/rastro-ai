#!/usr/bin/env bash
# Checks scripts/affected-tests.sh --dry-run against the real workspace graph.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
expect() {
  local want=$1; shift
  local got
  got=$(scripts/affected-tests.sh --dry-run "$@" 2>&1) || true
  if [ "$got" != "$want" ]; then
    echo "FAIL: $* -> '$got' (want '$want')"
    fail=1
  else
    echo "ok: $* -> $got"
  fi
}

# A leaf crate selects only itself.
expect "rastro-cli" crates/rastro-cli/src/main.rs
# A core crate selects itself and every transitive dependent.
expect "rastro-cli rastro-core rastro-operations rastro-storage-sqlite" crates/rastro-core/src/lib.rs
# A crate several others build on, reached through a directory path.
expect "rastro-adapter-claude-code rastro-cli rastro-connector-github rastro-connector-linear rastro-core rastro-extractor-github rastro-extractor-linear rastro-operations rastro-plugin-sdk rastro-storage-sqlite rastro-test-connector rastro-test-extractor" crates/rastro-plugin-sdk/
# Two paths union their sets.
expect "rastro-cli rastro-connector-linear" crates/rastro-cli/src/a.rs crates/rastro-connector-linear
# Global files select the whole workspace.
expect "workspace" Cargo.toml
expect "workspace" Cargo.lock docs/concept.md
expect "workspace" rust-toolchain.toml
# Docs and other non-crate paths select nothing.
expect "nothing selected" docs/concept.md README.md
exit $fail
