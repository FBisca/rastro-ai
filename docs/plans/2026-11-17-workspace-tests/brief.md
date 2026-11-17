# Brief: workspace tests

Slug: `workspace-tests`. Contract: `plan.yaml`. Lane: Plan, big × complex: the test runner of every crate, CI and the check tiers change, and the first ticket fixes a concurrency defect.

## What changes

- `crates/rastro-cli/tests/support/binaries.rs`: a test's binary snapshot can vanish under process-per-test, failing 10 tests under nextest → each test's snapshot survives until it finishes (T1)
- CI, `.orchestrate/config.yaml`, `scripts/affected-tests.sh`: `cargo test` → `cargo nextest run`, a test SLOW at 20s and killed at 60s, JUnit failure output under `target/nextest/<profile>/`, recorded runs rerun with `--rerun latest`; ADR-0013 records the runner (T2)
- Ignored tests: free-text reasons → `live:` or `bench:` reasons naming their env vars, refused otherwise; a `live` profile runs them alone (T3)
- `src/` tests in core's host, operations' catalog and storage's lib that touch the filesystem or a process → moved under `tests/`; a `fast` profile, the fast check tier, runs the unit tests alone (T4)

## Decisions

D1 cargo-nextest runs the tests, over `cargo test` alone: per-test timeouts, JUnit output and reruns of failures, built for large suites; recorded as ADR-0013, `proposed` until this plan is approved (owner, 2026-11-17; [JetBrains](https://blog.jetbrains.com/rust/2026/05/01/faster-rust-tests-with-cargo-nextest/)).
D2 The workspace has no doctests (0 across 4 doctest binaries), so nextest skipping doctests loses nothing today (gather, `cargo test --workspace --doc`).
D3 Under nextest 0.9.151, 10 tests fail that pass under `cargo test` (github 35/35, the mcp and sync_search singles green), panicking at `crates/rastro-cli/tests/support/binaries.rs:18`; the snapshot fix lands before the switch (run, 2026-11-17).
D4 nextest 0.9.151 reruns the failures of a recorded run with `--rerun latest` (run, `cargo nextest run --help`).
D5 A test's location is its level: `src/` `#[cfg(test)]` is unit (no process, filesystem or socket) and runs as the fast suite; `tests/` is integration, over per-file nextest overrides: Rust has no markers, and a per-file table rots (owner, 2026-11-17).
D6 Live tests stay `#[ignore]` with a reason starting `live:` that names their env vars (the benchmark `bench:`), run by a `live` profile with `--run-ignored only`, over one new opt-in variable: the existing variables carry the repo, phrase and database (owner, 2026-11-17).
D7 A test is SLOW at 20s and killed at 60s, over agent-runtime's 18s: the slowest test took 27s under full parallel load (owner, 2026-11-17).
D8 Each run writes JUnit with failure output and is recorded; agents read the JUnit before re-running anything (owner, 2026-11-17).
D9 The gates' requirements live in `docs/specs/workspace/`, one file per area (`tests.md`, `specs.md`, `crates.md`), one plan per file, over one `workspace-contracts.md`: a spec may be a folder of files per domain (owner, 2026-11-17).
