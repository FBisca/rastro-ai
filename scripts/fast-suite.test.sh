#!/usr/bin/env bash
# Checks the fast suite: `cargo nextest run --workspace --profile fast` runs every unit test, the
# ones in a `src/` module, and no test from a `tests/` binary. The check config's fast tier runs the
# same command on every hand-in, and the affected selector and CI keep the whole suite.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

profile=fast

fail=0
failures=0
before=0

start_test() {
  echo "test: $1"
  before=$failures
}

# end_test compares the failure count, so a test that fails is never reported ok because an earlier
# test already failed.
end_test() {
  if [ "$failures" -eq "$before" ]; then echo "ok: $1"; else echo "FAIL: $1"; fi
}

# flunk <message>: notes a failure of the current test.
flunk() {
  echo "  $1"
  fail=1
  failures=$((failures + 1))
}

# selection [<nextest list option>...]: prints one line per test nextest would run, as the binary's
# kind, its binary id, the test's name and whether it is ignored, separated by tabs. The kind is
# `lib` or `bin` for a unit test and `test` for a `tests/` binary.
selection() {
  local listed errors
  errors=$(mktemp)
  # nextest writes cargo's build lines to stderr: keep them out of the JSON, and show them when the
  # listing itself fails.
  if ! listed=$(cargo nextest list --workspace --color never \
    --message-format json "$@" 2>"$errors"); then
    cat "$errors" >&2
    rm -f "$errors"
    return 1
  fi
  rm -f "$errors"
  printf '%s\n' "$listed" | jq -r '
    ."rust-suites" | to_entries[]
    | .key as $binary
    | .value.kind as $kind
    | .value.testcases | to_entries[]
    | [$kind, $binary, .key, (.value.ignored | tostring)] | @tsv'
}

# units: from a selection, the lines of the unit tests alone, as "<binary id> <test name>".
units() {
  awk -F'\t' '$1 == "lib" || $1 == "bin" { print $2 " " $3 }' | sort
}

# integration: from a selection, the lines of the `tests/` binaries' tests alone.
integration() {
  awk -F'\t' '$1 == "test" { print $2 " " $3 }' | sort
}

# The whole workspace and the fast profile's selection, listed once each; the run below is the same
# command the fast tier runs.
all=$(selection)
all_status=$?
fast=$(selection --profile "$profile")
fast_status=$?
run_output=$(cargo nextest run --workspace --color never --profile "$profile" 2>&1)
run_status=$?

# spec: A test's location is its level
# Every unit test the workspace holds runs under the fast profile, and the run starts them all.
test_the_fast_profile_runs_every_unit_test() {
  start_test the_fast_profile_runs_every_unit_test
  local expected actual
  if [ "$all_status" -ne 0 ]; then
    flunk "cargo nextest list --workspace failed; the workspace's tests can't be listed"
    end_test the_fast_profile_runs_every_unit_test
    return
  fi
  if [ "$fast_status" -ne 0 ]; then
    flunk "cargo nextest list --profile $profile failed; the fast profile does not select tests"
    end_test the_fast_profile_runs_every_unit_test
    return
  fi
  expected=$(printf '%s\n' "$all" | units)
  actual=$(printf '%s\n' "$fast" | units)
  if [ -z "$expected" ]; then
    flunk "nextest lists no test in a lib or bin target, so the check proves nothing"
    end_test the_fast_profile_runs_every_unit_test
    return
  fi
  local missing extra
  missing=$(comm -23 <(printf '%s\n' "$expected") <(printf '%s\n' "$actual"))
  if [ -n "$missing" ]; then
    flunk "the fast profile does not run every unit test; these are missing:"
    printf '%s\n' "$missing" | sed 's/^/    /'
  fi
  extra=$(comm -13 <(printf '%s\n' "$expected") <(printf '%s\n' "$actual"))
  if [ -n "$extra" ]; then
    flunk "the fast profile runs tests that are not unit tests:"
    printf '%s\n' "$extra" | sed 's/^/    /'
  fi
  # The real run, not the listing: the profile is what the fast tier runs, and it must end green.
  if [ "$run_status" -ne 0 ]; then
    flunk "cargo nextest run --workspace --profile $profile exited $run_status"
    printf '%s\n' "$run_output" | tail -15 | sed 's/^/    /'
  else
    local expected_count started
    expected_count=$(printf '%s\n' "$fast" | awk -F'\t' '$4 == "false"' | wc -l | tr -d ' ')
    started=$(printf '%s\n' "$run_output" | grep -oE 'Starting [0-9]+ tests? across' | head -1 | grep -oE '[0-9]+')
    if ! printf '%s\n' "$run_output" | grep -q "with nextest profile: $profile"; then
      flunk "the run does not say it used the $profile profile:"
      printf '%s\n' "$run_output" | grep 'nextest profile' | sed 's/^/    /'
    fi
    if [ "$started" != "$expected_count" ]; then
      flunk "the run started ${started:-no} tests where the profile selects $expected_count"
    fi
  fi
  end_test the_fast_profile_runs_every_unit_test
}

# spec: A test's location is its level
# No test from a `tests/` binary is selected, and the workspace has one, so the check proves
# something.
test_the_fast_profile_runs_no_test_from_a_tests_binary() {
  start_test the_fast_profile_runs_no_test_from_a_tests_binary
  if [ "$fast_status" -ne 0 ]; then
    flunk "cargo nextest list --profile $profile failed, so the selection is unknown"
    end_test the_fast_profile_runs_no_test_from_a_tests_binary
    return
  fi
  local foreign
  foreign=$(printf '%s\n' "$fast" | integration)
  if [ -n "$foreign" ]; then
    flunk "the fast profile runs tests from a tests/ binary:"
    printf '%s\n' "$foreign" | sed 's/^/    /'
  fi
  # Without a `tests/` binary in the workspace the clause above would hold for any profile, so the
  # workspace must have integration tests for this check to mean anything.
  if [ "$all_status" -eq 0 ] && [ -z "$(printf '%s\n' "$all" | integration)" ]; then
    flunk "the workspace lists no test from a tests/ binary, so the check proves nothing"
  fi
  end_test the_fast_profile_runs_no_test_from_a_tests_binary
}

test_the_fast_profile_runs_every_unit_test
test_the_fast_profile_runs_no_test_from_a_tests_binary

exit $fail
