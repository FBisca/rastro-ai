#!/usr/bin/env bash
# Checks scripts/affected-tests.sh against the real workspace graph: the --dry-run selection, and the
# nextest invocation it runs for the selection.
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
expect "rastro-action-linear rastro-adapter-claude-code rastro-cli rastro-connector-github rastro-connector-linear rastro-core rastro-extractor-github rastro-extractor-linear rastro-operations rastro-plugin-sdk rastro-storage-sqlite rastro-test-connector rastro-test-extractor" crates/rastro-plugin-sdk/
# Two paths union their sets.
expect "rastro-cli rastro-connector-linear" crates/rastro-cli/src/a.rs crates/rastro-connector-linear
# Global files select the whole workspace.
expect "workspace" Cargo.toml
expect "workspace" Cargo.lock docs/concept.md
expect "workspace" rust-toolchain.toml
# Docs and other non-crate paths select nothing.
expect "nothing selected" docs/concept.md README.md

# spec: Tests run under nextest
# The selector runs the selected crates' tests under nextest: one -p per crate, or --workspace for a
# path that touches every crate. A cargo stand-in records the invocation instead of running the tests.
stub=$PWD/scripts/fixtures/nextest/cargo
log=$(mktemp)
trap 'rm -f "$log"' EXIT

expect_run() { # want-command, paths...
  local want=$1; shift
  local got
  : > "$log"
  CARGO=$stub NEXTEST_FIXTURE_REAL_CARGO=$(command -v cargo) NEXTEST_FIXTURE_LOG=$log \
    scripts/affected-tests.sh "$@" >/dev/null 2>&1 || true
  got=$(cat "$log")
  if [ "$got" != "$want" ]; then
    echo "  run $* -> '$got' (want '$want')"
    fail=1
  else
    echo "  ok: run $* -> $got"
  fi
}

test_the_selector_runs_the_selected_crates_under_nextest() {
  echo "test: the_selector_runs_the_selected_crates_under_nextest"
  local before=$fail
  expect_run "nextest run -p rastro-cli" crates/rastro-cli/src/main.rs
  expect_run "nextest run --workspace" Cargo.toml
  expect_run "nextest run -p rastro-cli -p rastro-connector-linear" \
    crates/rastro-cli/src/a.rs crates/rastro-connector-linear
  expect_run "" docs/concept.md README.md
  if [ "$fail" -eq "$before" ]; then
    echo "ok: the_selector_runs_the_selected_crates_under_nextest"
  else
    echo "FAIL: the_selector_runs_the_selected_crates_under_nextest"
  fi
}

test_the_selector_runs_the_selected_crates_under_nextest
exit $fail
