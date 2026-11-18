### Requirement: Tests run under nextest
CI, the orchestrate check tiers and the affected selector SHALL run the workspace's tests with cargo-nextest, each test in its own process.

#### Scenario: the whole suite passes process-per-test
- WHEN CI or the full check tier runs the workspace's tests
- THEN it runs `cargo nextest run --workspace`, and every test that passes under `cargo test --workspace` passes there

#### Scenario: the affected selector runs nextest
- WHEN scripts/affected-tests.sh runs for paths that select crates
- THEN it invokes `cargo nextest run` with one `-p` per selected crate, or `--workspace` for a global path, and its `--dry-run` selection is unchanged

### Requirement: A test runs its own binary snapshot
A test that runs a workspace binary SHALL run a private snapshot that no other test process removes or replaces while it runs.

#### Scenario: many test processes snapshot and sweep the same binaries at once, as nextest runs them
- WHEN many test processes snapshot and sweep the same binaries at once, as nextest runs them
- THEN every test runs its own snapshot to completion, and none panics on a missing snapshot or binary

### Requirement: A test's location is its level
A test in a `src/` `#[cfg(test)]` module SHALL be a unit test that starts no process and touches no filesystem or socket, a test under `tests/` SHALL be an integration test, and an ignored test SHALL be a live or bench test by its reason.

#### Scenario: the fast suite is the unit tests
- WHEN `cargo nextest run --profile fast` runs
- THEN every `src/` unit test runs, and no test from a `tests/` binary runs

#### Scenario: an ignored test says why
- WHEN an `#[ignore]` carries no reason, or a reason that does not start with `live:` or `bench:`
- THEN the ignore check exits non-zero naming the file and line

#### Scenario: live tests only on request
- WHEN the default profile runs, and when the `live` profile runs
- THEN the default run skips every ignored test, and the `live` run runs the ignored ones alone

### Requirement: A hung test fails
The default profile SHALL report a test running past 20 seconds as SLOW and SHALL kill and fail a test running past 60 seconds.

#### Scenario: a test runs past 60 seconds under the repository's nextest config
- WHEN a test runs past 60 seconds under the repository's nextest config
- THEN nextest terminates it, names it as timed out, and the run exits non-zero

### Requirement: The last run's failures are kept
Every nextest run SHALL leave a JUnit report holding each failing test with its output, and a recording from which only the failures rerun.

#### Scenario: a run has a failing test
- WHEN a run has a failing test
- THEN `target/nextest/<profile>/junit.xml` names that test and holds its captured output

#### Scenario: `cargo nextest run --rerun latest` follows a run with failures
- WHEN `cargo nextest run --rerun latest` follows a run with failures
- THEN only the tests that failed or did not complete run again
