#!/usr/bin/env bash
# Checks the ignore-reason gate and the repository's live profile, against fixture trees and real
# nextest runs.
#
# The fixtures are written under target/, so this script needs nothing committed and leaves the
# developer's own runs alone.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

repo=$PWD
config=$repo/.config/nextest.toml
fixtures=$repo/target/ignored-tests-fixtures

# CI sets CARGO_TERM_COLOR, and nextest colours its output when it is set; escape codes then split
# the lines these checks match. Every run below asks for plain output so the checks read the text.
export NEXTEST_HIDE_PROGRESS_BAR=1

rm -rf "$fixtures"

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

# says <haystack> <what it is> <needle>: notes a missing needle and fails.
says() {
  case $1 in
    *"$3"*) ;;
    *) flunk "$2 does not hold '$3'" ;;
  esac
}

# says_not <haystack> <what it is> <needle>: notes an unexpected needle and fails.
says_not() {
  case $1 in
    *"$3"*) flunk "$2 holds '$3'" ;;
  esac
}

# spec: an ignored test says why
# The fixtures hold one attribute each, so the check has to name that file and line; a reason that
# reads well but names no kind passes nothing.
test_an_ignore_without_a_live_or_bench_reason_is_refused() {
  start_test an_ignore_without_a_live_or_bench_reason_is_refused
  local out status

  mkdir -p "$fixtures/no-reason/src"
  cat > "$fixtures/no-reason/src/no_reason.rs" <<'RS'
#[test]
#[ignore]
fn a_live_check() {}
RS
  out=$(scripts/ignored-tests.sh --root "$fixtures/no-reason" 2>&1)
  status=$?
  if [ "$status" -eq 0 ]; then
    flunk "an #[ignore] with no reason exited 0"
  fi
  says "$out" "the check" "src/no_reason.rs:2"

  mkdir -p "$fixtures/unprefixed/src"
  cat > "$fixtures/unprefixed/src/unprefixed.rs" <<'RS'
#[test]
#[ignore = "needs a copy of the owner's database"]
fn a_live_check() {}
RS
  out=$(scripts/ignored-tests.sh --root "$fixtures/unprefixed" 2>&1)
  status=$?
  if [ "$status" -eq 0 ]; then
    flunk "an #[ignore] reason without live: or bench: exited 0"
  fi
  says "$out" "the check" "src/unprefixed.rs:2"

  end_test an_ignore_without_a_live_or_bench_reason_is_refused
}

# spec: an ignored test says why
# Both kinds are reasons: a live test talks to a service and a bench test measures a release build.
test_a_live_or_bench_reason_is_accepted() {
  start_test a_live_or_bench_reason_is_accepted
  local out status

  mkdir -p "$fixtures/accepted/src"
  cat > "$fixtures/accepted/src/accepted.rs" <<'RS'
#[test]
#[ignore = "live: talks to GitHub; set RASTRO_LIVE_REPO"]
fn a_live_check() {}

#[test]
#[ignore = "bench: release build only"]
fn a_bench_check() {}
RS
  out=$(scripts/ignored-tests.sh --root "$fixtures/accepted" 2>&1)
  status=$?
  if [ "$status" -ne 0 ]; then
    flunk "a live: and a bench: reason exited $status:"
    printf '%s\n' "$out"
  fi

  # The repository itself passes: every ignore in it names its kind.
  out=$(scripts/ignored-tests.sh 2>&1)
  status=$?
  if [ "$status" -ne 0 ]; then
    flunk "the repository's own ignores exit $status:"
    printf '%s\n' "$out"
  fi

  end_test a_live_or_bench_reason_is_accepted
}

# spec: live tests only on request
# A live test costs a network round trip or the owner's data, so the run everyone makes must skip it
# and the live run must hold nothing else. The fixture workspace has one test of each kind, so either
# run that takes the other one shows in its own output.
test_the_default_run_skips_the_ignored_tests_the_live_profile_runs_them_alone() {
  start_test the_default_run_skips_the_ignored_tests_the_live_profile_runs_them_alone
  local fixture=$fixtures/live-profile out status

  mkdir -p "$fixture/src"
  cat > "$fixture/Cargo.toml" <<'TOML'
[package]
name = "ignored-tests-fixture"
version = "0.0.0"
edition = "2024"
publish = false

# Its own workspace: the fixture must not join the repository's one.
[workspace]
TOML
  cat > "$fixture/src/lib.rs" <<'RS'
//! A fixture workspace with one test that runs by default and one live test.
#[cfg(test)]
mod tests {
    #[test]
    fn test_that_runs_by_default() {
        println!("fixture: the default test ran");
    }

    #[test]
    #[ignore = "live: needs a service this machine does not reach"]
    fn test_live_that_needs_a_service() {
        println!("fixture: the live test ran");
    }
}
RS

  out=$(cd "$fixture" && cargo nextest run --color never --config-file "$config" 2>&1)
  status=$?
  if [ "$status" -ne 0 ]; then
    flunk "the default run exited $status:"
    printf '%s\n' "$out"
  fi
  says "$out" "the default run" "1 test run: 1 passed"
  says "$out" "the default run" "1 test skipped"
  says_not "$out" "the default run" "test_live_that_needs_a_service"

  out=$(cd "$fixture" && cargo nextest run --color never --profile live --run-ignored only \
    --config-file "$config" 2>&1)
  status=$?
  if [ "$status" -ne 0 ]; then
    flunk "the live run exited $status:"
    printf '%s\n' "$out"
  fi
  says "$out" "the live run" "1 test run: 1 passed"
  says "$out" "the live run" "test_live_that_needs_a_service"
  says_not "$out" "the live run" "test_that_runs_by_default"

  # A live test exists to print what it saw: an answer, a citation, a p95. nextest hides a passing
  # test's output by default, so the live run shows it, and its report keeps it.
  says "$out" "the live run" "fixture: the live test ran"
  local report=$fixture/target/nextest/live/junit.xml
  if [ ! -f "$report" ]; then
    flunk "no JUnit report at $report"
  else
    says "$(cat "$report")" "the live JUnit report" "fixture: the live test ran"
  fi

  end_test the_default_run_skips_the_ignored_tests_the_live_profile_runs_them_alone
}

# spec: live tests only on request
# The live run is one command, `cargo nextest run --profile live --run-ignored only`: nextest reads the
# selection from the flag, and the profile gives that run its settings. A form that drops either half
# runs the wrong tests with the wrong timeouts, so every place that writes a live run — the ignore
# reasons, the docs, the config — names the profile.
test_every_live_run_is_the_documented_command() {
  start_test every_live_run_is_the_documented_command
  local out file files=(.config/nextest.toml docs/development.md)
  # The places that write a run for an operator: the config, the guide and the ignore reasons. The
  # plan's own files predate the decision and a ticket may not edit them, so they are not read here.
  while IFS= read -r file; do files+=("$file"); done < <(find crates -name '*.rs' -type f)
  out=$(grep -nIH -- '--run-ignored' "${files[@]}" | grep -v -- '--profile live')
  if [ -n "$out" ]; then
    flunk "these write a live run other than the documented one:"
    printf '%s\n' "$out"
  fi
  end_test every_live_run_is_the_documented_command
}

test_an_ignore_without_a_live_or_bench_reason_is_refused
test_a_live_or_bench_reason_is_accepted
test_the_default_run_skips_the_ignored_tests_the_live_profile_runs_them_alone
test_every_live_run_is_the_documented_command

exit $fail
