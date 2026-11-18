# Rust development

Read the relevant spec and accepted ADRs before changing a boundary. Use the crate roles in [README.md](../README.md#how-its-built) to locate ownership; keep domain behavior in the core, storage behind its interfaces, and surface translation in the CLI/MCP and operation catalog.

## Standards

- Follow nearby Rust conventions and the pinned toolchain. Let rustfmt own formatting and Clippy own lint diagnostics; keep lint exceptions local with a reason.
- Represent domain distinctions with types and enums; validate external data at the boundary. Preserve the glossary's meanings in names and public interfaces.
- Return recoverable failures with `Result` and propagate with `?`; add actionable context using the existing `anyhow` conventions. Reserve production `expect`/`unwrap` for documented invariants, never malformed input, plugin failures, or I/O. Test setup may use them.
- Borrow inputs when ownership is unnecessary. Add cloning, shared ownership, concurrency, or abstractions for a concrete requirement rather than a hypothetical extension.
- Keep new code safe Rust. Any necessary `unsafe` needs a documented safety invariant and focused evidence that callers uphold it.
- Reuse workspace dependencies; put a dependency in the crate that needs it. Keep `Cargo.lock` updated deliberately and explain new dependencies in the PR. A library or storage replacement still follows the ADR process.
- Keep protocol stdout machine-readable and diagnostics on stderr. Preserve operation envelopes, scope filtering, evidence provenance, and replay semantics when extending their boundaries.
- Document new public contracts, including failure conditions and invariants. Use runnable rustdoc examples where they clarify a caller's use.

## Crate roles

Every workspace member declares the role it plays, in its own manifest:

```toml
[package.metadata.rastro]
role = "core"
```

[`scripts/crate-gate.sh`](../scripts/crate-gate.sh) reads those roles and fails when a member declares none — no metadata, a `rastro` value that is not a table, or no `role` under it — declares one the table does not name, depends — normal, dev or build — on a member whose role its own role does not allow, or has no `[lints] workspace = true` to inherit the workspace lints. The table lives in that script:

| Role | Crates | May depend on |
|---|---|---|
| `sdk` | `rastro-plugin-sdk` | nothing |
| `core` | `rastro-core` | `sdk` |
| `storage` | `rastro-storage-sqlite` | `core`, `sdk` |
| `operations` | `rastro-operations` | `core`, `sdk` |
| `surface` | `rastro-cli` | `core`, `operations`, `storage`, `sdk` |
| `plugin` | `rastro-connector-*`, `rastro-extractor-*`, `rastro-adapter-claude-code`, `rastro-test-*` | `sdk` |

An edge the table does not allow is a design change, not a local one: amend the table in the script and here in the same commit, and say why in the pull request. Run the gate from the workspace root, or point it at another workspace with `--root <dir>`, which is how [`scripts/crate-gate.test.sh`](../scripts/crate-gate.test.sh) checks it against the fixture workspaces in `scripts/fixtures/crate-gate/`. The gate needs Cargo and jq, like the affected selector.

## Tests run under nextest

The workspace's tests run under [cargo-nextest](https://nexte.st), one process per test, configured by [.config/nextest.toml](../.config/nextest.toml): CI, the orchestrator's full tier and the affected selector all run `cargo nextest run`. Install it once with `cargo install cargo-nextest --locked`, or with your package manager.

```sh
cargo nextest run --workspace                     # the whole suite, as CI runs it
cargo nextest run --workspace --profile fast      # the unit tests alone, as the fast tier runs them
cargo nextest run -p <package> --lib              # one crate's unit tests
cargo nextest run -p <package> --test <target>    # one test in crates/<package>/tests/<target>.rs
```

The config reports a test running past 20 seconds as SLOW and kills it after three periods (60 seconds), never retries a failure, and keeps going after one so a single run holds every failure: `target/nextest/default/junit.xml` names each failing test with its captured output. See [ADR 0013](adr/0013-cargo-nextest-runs-the-tests.md).

### The fast suite

A test's location is its level. A test in a `src/` `#[cfg(test)]` module is a unit test: it starts
no process and touches no filesystem or socket. A test that needs a database file, a spawned
process, a network, or a built workspace binary is an integration test under the crate's `tests/`,
where nextest runs it in its own process beside the rest of the suite.

The `fast` profile runs the unit tests alone — the lib and bin targets of every crate, no `tests/`
binary:

```sh
cargo nextest run --workspace --profile fast
```

The orchestrator's fast check tier runs the same command on every hand-in, so a change that breaks a
unit test fails the cheapest tier first. [`scripts/fast-suite.test.sh`](../scripts/fast-suite.test.sh)
proves the selection — every unit test, no test from a `tests/` binary — and that the run is green.
CI runs the whole suite under the default profile, and the affected selector runs the selected crates' tests plus their dependents.

### Live and bench tests

An ignored test is one the default run skips, and it earns that place only as a live test or a bench test: its `#[ignore]` reason starts with `live:` or `bench:`, and a live test names every environment variable it reads, so an operator can run it without reading the test.

```rust
#[ignore = "live: reads a copy of the owner's database; set RASTRO_LIVE_DB and RASTRO_LIVE_ENTITY"]
#[ignore = "bench: release build only"]
```

[`scripts/ignored-tests.sh`](../scripts/ignored-tests.sh) reads every `#[ignore]` in the workspace and fails, naming the file and line, on one whose reason is missing or starts with anything else. An ignored test that says none of the two is a test nobody runs.

Live tests talk to a real service or read a copy of the owner's data, so the default run never touches them: name every variable they read, and run them on request with the live run. The live run is one command, with no other form:

```sh
cargo nextest run --profile live --run-ignored only
```

Both halves are part of it, not one instead of the other: nextest reads the selection from the flag in that command, and the `live` profile gives that run a network round trip's timeouts instead of the default run's 20 and 60 seconds, each test's output as it lands, and a JUnit report at `target/nextest/live/junit.xml` that keeps a passing test's output too. A live test exists to print what it saw — an answer, a citation, a p95 — so the live run shows those figures where the default run keeps a passing test quiet.

A bench test measures a release build, so its live run adds `--release`:

```sh
cargo nextest run --profile live --release --run-ignored only
```

### Rerun only the last run's failures

nextest's record-replay-rerun feature is experimental and reads the user config file, not the repository's, so point `NEXTEST_USER_CONFIG_FILE` at [.config/nextest-user.toml](../.config/nextest-user.toml) in your shell profile, or copy that file to `~/.config/nextest/config.toml`. Every run then leaves a recording:

```sh
cargo nextest run --workspace
cargo nextest run --rerun latest   # only the tests that failed or did not complete
```

CI sets the same variable, so it records its runs too. `scripts/nextest-config.test.sh` checks the config, both commands and the gates.

## Red → green

Use this loop for behavior changes and bug fixes. When `/tdd` is available, use its admission rules; this guide also works without a personal skill installation.

1. Identify the observable behavior and public seam in the task or approved plan. If neither names the seam, agree it before adding tests. Search existing coverage; extend or parameterize a test where it already protects the same guarantee.
2. Pick the cheapest reliable level: unit for local decisions, integration for storage/process/protocol boundaries, a small number of end-to-end journeys. Use existing scripted plugins and local API fixtures for external services; keep tests independent of live accounts, credentials, wall-clock waits, and each other's state.
3. Run one focused test and observe the intended behavioral failure. A compiler error or zero matched tests is not red. Implement the smallest change that makes that test green, then repeat for the next behavior.
4. After the behavior is green, review the implementation and run the affected selection below. Add a regression test only for an uncovered, plausible failure; documentation-only edits need document checks rather than new tests.

## Tight feedback

Run from the workspace root. Replace placeholders with names from the crate and its test targets:

```sh
# One library test; `test(=...)` matches the name exactly.
cargo nextest run -p <package> --lib -E 'test(=module::test_name)'
# One test in crates/<package>/tests/<target>.rs.
cargo nextest run -p <package> --test <target> -E 'test(=test_name)'
# Discover names when a filter matches nothing.
cargo nextest list -p <package> --lib
cargo nextest list -p <package> --test <target>
```

Confirm the intended test actually ran. `-p` limits packages; a name filter alone still builds the selected targets. Use `--lib` or `--test` to narrow compilation during the loop. `cargo nextest run -p <package>` restores that package's default targets; nextest does not run doctests, so run `cargo test --doc -p <package>` for a crate that documents one. See [nextest's filtersets](https://nexte.st/docs/filtersets/).

## Affected checks

Cargo has no built-in `vitest --changed` equivalent. The repository's [affected selector](../scripts/affected-tests.sh) accepts paths and runs, under `cargo nextest run`, the tests of their crates plus transitive workspace dependents, including dev-dependencies. It requires Bash, Cargo, jq and cargo-nextest. Inspect before running:

```sh
scripts/affected-tests.sh --dry-run crates/rastro-core/src/lib.rs
scripts/affected-tests.sh crates/rastro-core/src/lib.rs
```

To select all branch changes plus staged, unstaged, deleted, and untracked files, run this in **Bash**, from the workspace root, after fetching the PR base (`origin/main` here):

```bash
base=$(git merge-base HEAD origin/main) || exit 1
paths=()
while IFS= read -r -d '' path; do
  paths+=("$path")
done < <(
  git diff --name-only --no-renames -z "$base" --
  git ls-files --others --exclude-standard -z
)
scripts/affected-tests.sh --dry-run "${paths[@]}"
scripts/affected-tests.sh "${paths[@]}"
```

The NUL-delimited list preserves spaces; disabling rename detection includes both old and new paths. Root manifests, lockfile, toolchain, and `.cargo/` changes select the whole workspace. Other paths outside crates select nothing: inspect shared fixtures, schemas, scripts, and CI changes yourself and run their consumers or the workspace when impact is uncertain. The selector follows manifest dependencies, not runtime plugin launches or other implicit dependencies; include those consumers explicitly.

## Completion checks

- During iteration, run the focused test. Once green, run the affected selection once; rerun only after further relevant changes or a failure.
- For Rust changes, run `cargo fmt --all --check` and Clippy for the selected packages (`cargo clippy -p <package> --all-targets -- -D warnings`, repeat `-p` for dependents). Use workspace scope for global configuration or uncertain impact.
- For documentation-only changes, check links, examples, and `git diff --check`; skip Rust compilation unless executable examples or behavior changed.
- For selector changes, run `scripts/affected-tests.test.sh`. For role, manifest, or gate changes, run `scripts/crate-gate.sh` and `scripts/crate-gate.test.sh`. For changes to a test's location, an `#[ignore]`, [.config/nextest.toml](../.config/nextest.toml), [.config/nextest-user.toml](../.config/nextest-user.toml), the check tiers or the CI workflow, run `scripts/fast-suite.test.sh`, `scripts/ignored-tests.sh`, `scripts/ignored-tests.test.sh` and `scripts/nextest-config.test.sh`. For toolchain, dependency, shared build, or cross-workspace changes, run the full CI commands from [.github/workflows/ci.yml](../.github/workflows/ci.yml). CI remains the full workspace gate for every PR.
- Report exact commands and outcomes, including the red/green observation for behavior changes. Separate focused or affected evidence from a full workspace pass; state anything left to CI.
