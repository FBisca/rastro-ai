---
status: accepted
date: 2026-11-17
---

# cargo-nextest runs the tests: one process per test, a hung test killed, every failure kept

`cargo test` runs one binary's tests as threads of one process. A test that exits the process, spawns a process that outlives it, or writes to a shared file can fail tests it never touched; no test is ever timed out, so a hung test holds a run until CI's own limit; and the only record of a failure is console output from a run that has already finished. The workspace's tests are the shape that breaks this: plugins launched per test, SQLite files, built binaries snapshotted per test.

The repository runs its tests with [`cargo nextest`](https://nexte.st). One file, [`.config/nextest.toml`](../../.config/nextest.toml), holds the runner's behaviour; CI ([`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)), the orchestrator's full tier ([`.orchestrate/config.yaml`](../../.orchestrate/config.yaml)) and the affected selector ([`scripts/affected-tests.sh`](../../scripts/affected-tests.sh)) all run the same `cargo nextest run`, and [`scripts/nextest-config.test.sh`](../../scripts/nextest-config.test.sh) proves each clause below against the real binary.

## Shape

1. **One process per test.** nextest gives every test its own process, so a test that leaks a process, a file or a signal fails itself. Nothing in the repository opts back into a shared process: a test that needs several processes gets them itself, as the plugin tests already do.
2. **A hung test fails.** `slow-timeout = { period = "20s", terminate-after = 3 }`: a test past 20 seconds is reported SLOW, three periods in a row terminate it at 60 seconds, and the run exits non-zero naming the test as TIMEOUT. 20 seconds clears the slowest test on a loaded CI runner, and 60 seconds is short enough that a hung run fails while someone is still watching it.
3. **Every failure of a run is kept.** `fail-fast = false` keeps the run going, so one run holds every failure; `[profile.default.junit] path = "junit.xml"` writes `target/nextest/<profile>/junit.xml`, naming each failing test with its captured stdout and stderr. `retries = 0`: a test that only passes on a retry is a flake to fix, not a failure to hide.
4. **The last run's failures rerun without the suite.** nextest's record-replay-rerun feature is experimental and its configuration is read from the user config file only, never from the repository's; the repository therefore ships [`.config/nextest-user.toml`](../../.config/nextest-user.toml) (`[experimental] record = true`, `[record] enabled = true`) and expects `NEXTEST_USER_CONFIG_FILE` to point at it. CI sets it, and a developer can instead copy the file to `~/.config/nextest/config.toml`. Then `cargo nextest run --rerun latest` runs only the tests that failed or did not complete in the latest run. When the feature stabilises, its tables move into `.config/nextest.toml` and the user config file goes away.
5. **The gates are nextest's.** CI installs nextest and runs `cargo nextest run --workspace`; the full tier is the same command; the selector keeps its `--dry-run` selection and runs nextest with one `-p` per selected crate, or `--workspace` for a path that touches every crate. CI does not also run `cargo test --workspace`: the suite runs once, in one runner.

## Considered options

- **Keep `cargo test`.** It is the default and needs nothing installed, but it has no per-test timeout, no machine-readable report, no rerun, and no isolation between tests: the problems this solves are all of its design.
- **`cargo test -- --test-threads=1`.** Serialising hides the collisions instead of removing them: state is still shared, there is still no timeout and no JUnit report, and the run gets slower.
- **`cargo test` under a shell `timeout`.** That bounds the run, not the test: one hung test kills a run of hundreds and names nothing beyond the last line printed.
- **nextest behind a wrapper script.** A script would have to reimplement capture, the JUnit report and the rerun; nextest's `--profile`, `--message-format` and `--rerun` already are the interface those tools would wrap.
- **Enabling recording from the repository config.** Not possible in nextest 0.9.151: the `[experimental]` and `[record]` tables are user-level, and the repository config ignores them with a warning. Copying or pointing at a repository user config is the smallest honest step until the feature stabilises.

## Consequences

- `cargo test` no longer runs the suite here. [docs/development.md](../development.md) gives the nextest commands for a package, one test, and a rerun of the last failures; a filter that matches nothing is a mistake, not a pass.
- nextest does not run doctests, and the workspace has none today. A crate that adds one adds `cargo test --doc` to CI with it.
- The runner is now a tool to install (`cargo install cargo-nextest --locked`) and a version to keep: CI pins it, because recording is experimental and the config test asserts its exact timeout wording.
- Tests that leaned on a shared process now fail on their own; that is the point, and it is why the binary snapshot helper had to hand each test its own snapshot before this switch was safe.
- A test past 20 seconds shows up as SLOW in every run, so a suite that drifts slowly is visible before it hits the 60-second kill.
