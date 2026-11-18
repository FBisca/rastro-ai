# Contributing to Rastro

Rastro is a Rust workspace. This page gets you from a clone to a green change; the detail lives in
[docs/development.md](docs/development.md) (standards, the red → green loop, affected-test selection,
completion checks) and [AGENTS.md](AGENTS.md) (the rules for working in this repository).

## Setup

`rust-toolchain.toml` pins the toolchain (1.97.1 with rustfmt and clippy), so `rustup` installs it on
your first `cargo` command; the workspace itself needs nothing else. The affected-test selector needs
Bash, Cargo and `jq`; the spec gate needs Python 3.

```sh
cargo build --workspace          # every crate; plugins are separate binaries in the same workspace
cargo test -p rastro-core --lib  # one target at a time is the fast loop
python3 scripts/spec_coverage.py # specs and their tags agree
```

## The commands

| Command | What it holds |
|---|---|
| `cargo fmt --all --check` | Formatting |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lints |
| `cargo check --workspace --all-targets` | Types, every target |
| `cargo test --workspace` | The whole suite |
| `scripts/affected-tests.sh <path>...` | Tests for the crates your paths touch, and their dependents |
| `python3 scripts/spec_coverage.py` | Every titled requirement has a tagged test, and every tag a title |
| `python3 scripts/test_spec_coverage.py` | The spec gate's own tests |

`.github/workflows/ci.yml` runs, on every push and pull request, `scripts/affected-tests.test.sh` (the
selector's own tests), the spec gate and its tests, `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets -- -D warnings` and `cargo test --workspace`. `cargo check` and
`scripts/affected-tests.sh` are the quicker checks to run on your own change before you push; the
selector picks crates, so paths outside `crates/` — scripts, docs, workflows — select nothing and you
run their consumers yourself.

## The loop

1. Start from a requirement: a spec under `docs/specs/` states what the product does, one
   `### Requirement: <Title>` block at a time. A change that adds behaviour adds or extends one.
2. Write the test first, and watch it fail for the behaviour you expect rather than for a missing
   symbol.
3. Make the smallest change that passes it, then run the affected selection.
4. Tag the test with its requirement's exact title: `// spec: <Title>` or `/// spec: <Title>` on the
   line above `#[test]`, or `# spec: <Title>` above a test function — or above an `expect_… test_…`
   case — in a script test. Only a tag on a test covers a title, and every Rust `spec:` tag must name
   one a spec or an open plan holds. Run the spec gate after changing a requirement or a tag.
5. Commit as a Conventional Commit (`feat(core): ...`, `fix(cli): ...`) and open a pull request
   against `main`.

Seams, levels and completion checks are in [docs/development.md](docs/development.md); the product and
architecture rules, including the glossary and the ADR process, are in [AGENTS.md](AGENTS.md).
