# A kit for writing plugins outside the repository
Lane: Plan, regular and complex: a public protocol contract and a command that runs foreign plugins.

Rastro already loads plugins from outside the repository, but the protocol lives only in ADRs and Rust types, so an author in TypeScript or any other language has nothing to build or test against. This plan publishes a JSON Schema of the connector protocol and the manifest, adds `rastro plugin check` to run one synthetic run and name every violation, and writes an authoring guide. It is the foundation for the owner's first private plugin, which stays out of the repository. Two tickets, no other people.

## Your calls
None.

## What changes

### A plugin author
- **New:** `schema/plugin-protocol-v1.schema.json` describes the connector messages and the manifest, generated from the protocol library and kept equal to it by a test (S1); a real scripted session validates against it (S2).
- **New:** `rastro plugin check <id>` runs one synthetic run of an installed connector and prints the accepted counts when it conforms (S3), storing nothing (S4).
- **New:** it names each violation and exits unsuccessfully: a log line on stdout (S5), evidence of a kind the manifest doesn't permit (S6), no finish (S7).
- **New:** it warns about manifest kinds the core doesn't know without failing the run (S8).
- **New:** an authoring guide walks through the protocol and the TypeScript traps.

### Agents
- **New:** the check is CLI only; no MCP tool exposes it (S9).

## Walkthrough
1. An author writes a TypeScript connector from the schema and installs it under `<config>/plugins/work/` (S1).
2. `rastro plugin check work --option mailbox=me` runs it once against the real source (S3).
3. The connector logs with `console.log`: the check names the invalid stdout line and exits 1 (S5).
4. After switching logs to stderr, the check prints the accepted counts and exits 0; the home holds no evidence (S3, S4).

What can go wrong:
- The connector submits a kind it didn't declare: named, exit 1 (S6).
- It crashes before finishing: named as interrupted, exit 1 (S7).
- Its manifest says `kinds = ["connector", "mail"]`: a warning names `mail` (S8).

## Out of scope
- Extractor, action provider and adapter roles in the schema: later, by demand.
- Installing, approving permissions and revoking: slice 15.
- A TypeScript SDK package.
- Sandboxing a checked plugin.

## Risks
- **Running foreign code:** the check runs the author's plugin with its declared environment, the same trust a sync gives it; Rastro doesn't claim OS isolation (`docs/decisions/plugins.md` §13).
- **Schema drift:** the equality test fails on any difference (S1), and a recorded session must validate (S2).

---

# Reference

## Build order
- T1 schema derives, the generator, the committed schema and its two tests (regular · complex).
- T2 `rastro plugin check`, the authoring guide and decision §51 (regular · complex).

T1 and T2 can run in parallel.

## Decision log
- **D1 Recorded as a decision, not a roadmap slice.** A new `docs/decisions/plugins.md` §51; the README diagram does not change. Rejected: a new slice. Source: user, round 1 Q9.
- **D2 Schema v1 covers the connector role and the manifest.** `schema/plugin-protocol-v1.schema.json`, generated from `rastro-plugin-sdk` and kept current by an equality test. Rejected: every role at once. Why: the first user plugin is a connector; the operations schema already proves the pattern. Source: user, round 1 Q10; `docs/specs/surfaces.md` "The generated schema equals the committed file".
- **D3 `rastro plugin check <id>` runs one synthetic connector run.** Against the real source with the declared environment, storing nothing, with the same validation as a sync; prints each violation, warns on unknown manifest kinds, exits 1 on any violation; CLI only. Rejected: extending `rastro check`, which already means the credential probe. Source: user, round 1 Q11; `crates/rastro-cli/src/check.rs`.
- **D4 An authoring guide.** `docs/plugins/authoring.md`: the protocol walk-through and the traps for a TypeScript author (stdout logs fail the run, `node` must be on the basic PATH, secrets must be in `permissions.env`). Source: user, round 1 Q12; gather (`crates/rastro-core/src/host/mod.rs:437-447`).
- **D5 The synthetic run's binding.** Last 24 hours, first manifest scope, throwaway source name, command-line options. Rejected: storing into a real source. Source: user, approval round.

## Code touched
- `crates/rastro-plugin-sdk/src/lib.rs`: 3 of 61 types derive `JsonSchema` today (`Window`, `CoverageGap`, `Counts`); care points: `EvidenceSubmit.original` null-vs-absent, flattened maps, `Manifest.actions` renamed `action`.
- Pattern to copy: `crates/rastro-operations/src/schema.rs`, `src/bin/gen-schema.rs`, `tests/schema.rs`.
- Host: `crates/rastro-core/src/host/mod.rs` (`converse` loop, validation, limits; `launch` is private, so the check needs a seam that runs without storage).
- CLI: `crates/rastro-cli/src/main.rs` (subcommands Operation, Mcp, Tick, Check today).
- Fixtures: `crates/rastro-test-connector/` (scripted by `RASTRO_TEST_SCRIPT`), host tests in `crates/rastro-cli/tests/host.rs`.
- Spec: `docs/specs/surfaces.md` (added requirements).

## Interfaces

| Item | Value |
|---|---|
| Schema file | `schema/plugin-protocol-v1.schema.json`: `initialize` params and result, `evidence.submit`, `coverage.report`, `run.finish` and its counts, the manifest |
| Generator | a bin in `rastro-plugin-sdk`, same shape as `gen-schema` |
| Command | `rastro plugin check <id> [--option k=v]... [--json]` |
| Output | accepted counts; one line per violation with its reason (`protocol`, `interrupted`, `timeout`, a rejected kind, ...); warnings for unknown kinds |
| Exit | 0 with no violation, 1 with any |
