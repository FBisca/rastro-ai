#!/usr/bin/env bash
# Checks the repository's nextest configuration against the real nextest binary: the gates that run
# the suite, the timeout, the JUnit report and the recorded runs.
#
# The fixture projects under scripts/fixtures/nextest are their own Cargo workspaces, so each run
# points nextest at the repository's config with --config-file. Their builds and records are wiped
# first, so a run here neither reads nor clobbers a developer's own last run.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

repo=$PWD
fixtures=scripts/fixtures/nextest
config=$repo/.config/nextest.toml

# nextest keeps recorded runs in a per-user state directory; keep this run's out of the developer's.
export NEXTEST_STATE_DIR=$repo/target/nextest-config-test-state
# The repository's user-level config, which enables recording. CI points at the same file.
export NEXTEST_USER_CONFIG_FILE=$repo/.config/nextest-user.toml
export NEXTEST_HIDE_PROGRESS_BAR=1
rm -rf "$NEXTEST_STATE_DIR" "$fixtures/hang/target" "$fixtures/suite/target"

fail=0
before=$fail

start_test() {
  echo "test: $1"
  before=$fail
}

end_test() {
  if [ "$fail" -eq "$before" ]; then echo "ok: $1"; else echo "FAIL: $1"; fi
}

# says <haystack> <what it is> <needle>: notes a missing needle and fails.
says() {
  case $1 in
    *"$3"*) ;;
    *)
      echo "  $2 does not hold '$3'"
      fail=1
      ;;
  esac
}

# says_not <haystack> <what it is> <needle>: notes an unexpected needle and fails.
says_not() {
  case $1 in
    *"$3"*)
      echo "  $2 holds '$3'"
      fail=1
      ;;
  esac
}

# spec: Tests run under nextest
test_ci_and_the_full_tier_run_the_suite_under_nextest() {
  start_test ci_and_the_full_tier_run_the_suite_under_nextest
  local file contents
  for file in .github/workflows/ci.yml .orchestrate/config.yaml; do
    contents=$(cat "$file")
    says "$contents" "$file" "cargo nextest run --workspace"
    says_not "$contents" "$file" "cargo test --workspace"
  done
  end_test ci_and_the_full_tier_run_the_suite_under_nextest
}

# spec: A hung test fails
test_a_test_past_the_timeout_is_slow_terminated_and_named() {
  start_test a_test_past_the_timeout_is_slow_terminated_and_named
  local out status
  # The fixture sleeps past 90 seconds, so a config that never terminates it fails here instead of
  # passing by accident.
  out=$(cd "$fixtures/hang" && timeout 200 cargo nextest run --config-file "$config" 2>&1)
  status=$?
  if [ "$status" -eq 0 ]; then
    echo "  the run exited 0 with a hung test"
    fail=1
  fi
  says "$out" "the run" "SLOW"
  says "$out" "the run" "TERMINATING"
  # The summary line reads e.g. "TIMEOUT [  60.012s] nextest-hang-fixture::hang test_outlives_the_timeout".
  if ! printf '%s\n' "$out" | grep -q -E 'TIMEOUT \[ *60\.[0-9]+s\] .*test_outlives_the_timeout'; then
    echo "  no 60-second timeout naming the test; the run ended with:"
    printf '%s\n' "$out" | tail -5
    fail=1
  fi
  end_test a_test_past_the_timeout_is_slow_terminated_and_named
}

# spec: The last run's failures are kept
test_a_failing_test_reaches_the_junit_report_with_its_output() {
  start_test a_failing_test_reaches_the_junit_report_with_its_output
  local out status junit report
  out=$(cd "$fixtures/suite" && cargo nextest run --config-file "$config" 2>&1)
  status=$?
  if [ "$status" -eq 0 ]; then
    echo "  the run exited 0 with a failing test"
    fail=1
  fi
  # nextest writes a relative JUnit path under the workspace's target directory: under this fixture's
  # own target that is <workspace>/target/nextest/default/junit.xml, and for the repository's
  # workspace it is target/nextest/default/junit.xml.
  junit=$fixtures/suite/target/nextest/default/junit.xml
  if [ ! -f "$junit" ]; then
    echo "  no JUnit report at $junit; the run ended with:"
    printf '%s\n' "$out" | tail -5
    fail=1
  else
    report=$(cat "$junit")
    says "$report" "the JUnit report" 'testcase name="test_that_fails"'
    says "$report" "the JUnit report" "fixture: captured output before failing"
    # The second failure proves the run keeps going, so one JUnit report holds every failing test.
    says "$report" "the JUnit report" 'testcase name="test_that_also_fails"'
    says "$report" "the JUnit report" "fixture: second failure output"
  fi
  end_test a_failing_test_reaches_the_junit_report_with_its_output
}

# spec: The last run's failures are kept
test_a_rerun_runs_only_the_test_that_failed() {
  start_test a_rerun_runs_only_the_test_that_failed
  local out
  # The rerun needs a parent run with a failure of its own, so this test does not lean on another.
  (cd "$fixtures/suite" && cargo nextest run --config-file "$config") >/dev/null 2>&1
  out=$(cd "$fixtures/suite" && cargo nextest run --rerun latest --config-file "$config" 2>&1)
  says "$out" "the rerun" "test_that_fails"
  says "$out" "the rerun" "test_that_also_fails"
  says_not "$out" "the rerun" "test_that_passes"
  says "$out" "the rerun" "2 tests run"
  end_test a_rerun_runs_only_the_test_that_failed
}

test_ci_and_the_full_tier_run_the_suite_under_nextest
test_a_test_past_the_timeout_is_slow_terminated_and_named
test_a_failing_test_reaches_the_junit_report_with_its_output
test_a_rerun_runs_only_the_test_that_failed

exit $fail
