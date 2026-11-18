#!/usr/bin/env bash
# Checks scripts/crate-gate.sh against fixture workspaces and the real one.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
report_fail() {
  echo "FAIL: $*"
  fail=1
}

# expect_failure <test> <root> <fragment>...: the gate must exit non-zero and print every fragment.
expect_failure() {
  local test=$1 root=$2
  shift 2
  local out status=0
  out=$(scripts/crate-gate.sh --root "$root" 2>&1) || status=$?
  if [ "$status" = 0 ]; then
    report_fail "$test: exit 0, want non-zero (output: $out)"
    return
  fi
  local fragment
  for fragment in "$@"; do
    if ! grep -qF -- "$fragment" <<<"$out"; then
      report_fail "$test: output does not mention '$fragment' (output: $out)"
    fi
  done
}

# spec: Crates depend by role
# A role's allowed dependencies are the whole table: the plugin SDK may depend on no member, whether
# the dependency is normal or a dev-dependency.
expect_failure test_a_forbidden_edge_fails scripts/fixtures/crate-gate/forbidden-edge \
  "fixture-plugin-sdk (role sdk)" "fixture-core (role core)" "depends on fixture-tool (role core)"

# spec: Crates depend by role
# A member with no role at all is refused, not skipped.
expect_failure test_a_crate_without_a_role_fails scripts/fixtures/crate-gate/missing-role \
  "fixture-unspecified" "declares no role"

# spec: Crates depend by role
# A role the table does not name is refused, so a typo cannot pass as a known role.
expect_failure test_an_unknown_role_fails scripts/fixtures/crate-gate/unknown-role \
  "fixture-wizard" "wizard"

# spec: Crates depend by role
# A role that is not even a string is refused by name too, instead of aborting the gate's jq with an
# error that names no crate.
expect_failure test_a_role_that_is_not_a_string_fails scripts/fixtures/crate-gate/non-string-role \
  "fixture-numbered" "not in the role table"

# spec: Crates depend by role
# Metadata that is not a table declares no role, and is refused by name rather than aborting the
# gate's jq on a field that cannot exist.
expect_failure test_metadata_that_is_not_a_table_declares_no_role scripts/fixtures/crate-gate/non-table-metadata \
  "fixture-textual-metadata" "declares no role"

# spec: Every crate inherits the workspace lints
# A member that never writes `[lints] workspace = true` is refused by name, so a crate added without it
# fails before CI would run clippy with the workspace table missing.
expect_failure test_a_member_without_the_workspace_lints_fails scripts/fixtures/crate-gate/missing-lints \
  "fixture-unlinted" "does not inherit the workspace lints"

# spec: Crates depend by role
# This workspace passes: every member carries a role and every member dependency is allowed.
status=0
out=$(scripts/crate-gate.sh 2>&1) || status=$?
if [ "$status" != 0 ]; then
  report_fail "test_the_workspace_passes: exit $status (output: $out)"
fi

exit $fail
