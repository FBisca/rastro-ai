# Plan: Cited search over one real source (GitHub)

Slug: `cited-search-github`. Living spec: `docs/specs/evidence.md` (created when this plan is archived). Status: APPROVED 2026-11-04.

## Why now
Roadmap slice 1 is the walking skeleton for M1. Slice 2 (agents query Rastro over MCP) is the first time the owner uses Rastro as a client, and it needs this slice's core, store, plugin host and search. GitHub is a source the owner actually checks, so even slice 1 is usable on its own: cited search, plus honest gaps.

## Sources
| Source | What it settles |
|---|---|
| Owner request, 2026-11-04 session | Vertical slices with outcomes; ADRs for architecture; GitHub first; "involving the owner" = authored, assigned, review requested, @mentioned; accepted Codex's review changes |
| `docs/concept.md` §1, §4, §5, §7 | Cited evidence, honest gaps, everything is a plugin, library first, untrusted content can't steer |
| `docs/GLOSSARY.md` | Evidence, Source, Trust class, Coverage, Freshness rule, Scope, Connector, Plugin |
| `docs/roadmap.md` slice 1 | Scope and outcome of this slice |
| `docs/decisions/knowledge.md` §1–3, §11, §12, §14, §15, §16–20 | Three layers; evidence never changes; raw content default; coverage states and window; typed reasons; re-observation; empty search ≠ empty source |
| `docs/decisions/plugins.md` §1–4, §6, §9, §12–13 | Plugins request, the core decides; connector ≠ extractor; manifest; quarantine; run log; grants narrow; fixed read operations |
| `docs/decisions/surfaces.md` §1–2, §6, §8–10 | Core is a library; CLI and MCP share one schema; macOS and Linux; `--json`; data location |
| `docs/adr/0001`–`0005` | Rust core + out-of-process plugins; SQLite + FTS5 behind a trait; JSON-RPC over stdio; manifest and grants; observations and spans |
| `docs/research/knowledge-graphs-2026-11.md` | Storage ranking, FTS first, failed sync = unknown |
| Codex review (session scratchpad `codex-slice1-vision.md`) | Walkthrough, component map, risks, answers to the open questions |
| `agent-fleet` prototype `lib/fleet/brain.py` | Precedent: SQLite FTS5 (porter unicode61 remove_diacritics), `sync_state` as early coverage; upsert rejected |

## Decisions
| # | Choice | Rejected | Why | Source |
|---|---|---|---|---|
| D1 | First source: GitHub issues, PRs and their comments in one registered repo | Linear; vault folder; local folder | Real value, structured, auth already exists, not tied to one employer | Owner, 2026-11-04 |
| D2 | "Involving the owner" = authored ∪ assigned ∪ review requested ∪ @mentioned, the owner's login from `GET /user` | GitHub's `involves:` (also includes "commented") | Owner's definition | Owner, 2026-11-04 |
| D3 | API fields → trust class `structured`; titles, bodies and comments → `untrusted content` | Everything structured | Prose by others must never steer Rastro | knowledge §2; plugins §6 |
| D4 | SQLite + FTS5 behind a storage trait; the FTS index is a rebuildable projection | Graph DB; JSONL | Nothing to run or maintain; engines die | ADR 0002 |
| D5 | JSON-RPC 2.0 over stdio, versioned handshake, core-authoritative finish | gRPC; bare NDJSON | Language-neutral, MCP-shaped | ADR 0003 |
| D6 | `rastro-plugin.toml`; the manifest declares, the source grant narrows it and binds scope | Manifest as authorization | Permissions enforced by the core | ADR 0004; plugins §12 |
| D7 | Data location: `RASTRO_HOME` > XDG vars > `~/.local/share/rastro`, `~/.config/rastro` | `~/.rastro`; macOS `Library/` | One rule on both platforms, easy to isolate in tests | surfaces §10 |
| D8 | Evidence and coverage carry `scope` from the registered source; no grant enforcement beyond source binding until slice 14 | Defer the scope field | Evidence can't be rebuilt, so the field is needed now | knowledge §14 |
| D9 | Run log: plugin id and version, start, end, status and reason, counts of submitted, accepted, rejected and unchanged | Full cost and usefulness metrics now | Coverage needs runs; cost comes in slice 9 | plugins §9 |
| D10 | CLI: `source add`, `source list`, `sync [name]`, `search <text> [--history]`, `coverage [name]`, `index rebuild`; `--json` on all of them; connector-specific flags (`--repo`) | One generic `--path` | Matches the shared schema for MCP | surfaces §2, §9 |
| D11 | Coverage window: half-open `[from, to)` over `updated_at`, plus a selection; the first sync's `from` is set when the source is added (default: 90 days back); later syncs start where the last complete window ended | Cursor as coverage; full history on the first sync | Bounded, honest, resumable | knowledge §17 |
| D12 | Freshness rule default for GitHub: 1 h, configurable per source | Never stale | Past freshness, or after a latest failed attempt, the answer is "unknown" | knowledge §11 |
| D13 | Raw content: pointer + quoted span by default; `--keep-original` opt-in | Full original by default | Owner's choice per source | knowledge §3, §16 |
| D14 | Re-observation: unchanged → link the run, changed → append; search shows the latest observation per location, `--history` shows all | Upsert | Evidence never changes | ADR 0005 |
| D15 | The connector calls GitHub's REST API directly: token from `GH_TOKEN` or `gh auth token`, base URL overridable for tests, only fixed GET endpoints | Shelling out to `gh api` with built arguments | Fixed read operations; testable against a fixture server | plugins §13 |
| D17 | CI: GitHub Actions on `ubuntu-latest` and `macos-latest` running `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; toolchain pinned in `rust-toolchain.toml` | No CI until later | A ticket is COMPLETE only with green CI; macOS and Linux are required | surfaces §6; orchestrate gate |
| D18 | Rust test names carry the scenario ID in upper case (`fn S1_source_add_lists_the_source()`), with `#![allow(non_snake_case)]` at the top of each test file | Lower-case `s1_` names | The converge gate matches IDs case-sensitively | orchestrate gate |
| D16 | Cargo workspace: `rastro-core` (lib), `rastro-storage-sqlite` (lib), `rastro-cli` (bin `rastro`), `rastro-connector-github` (bin), `rastro-plugin-sdk` (lib: protocol types shared by connectors) | One crate | Library first; plugins depend on protocol types, never on the core | ADR 0001; Codex review |

## Scenarios
**Registration and location**
- **S1** WHEN the owner runs `rastro source add project --connector github --repo o/p --scope personal`, THEN `rastro source list` shows `project` with its connector, selector, scope, freshness rule and window start, and the config file holds it.
- **S2** WHEN a source is registered but has never synced, THEN `rastro coverage project` reports "never looked".
- **S3** WHEN `RASTRO_HOME` is set, THEN data and config live under it. Otherwise `XDG_DATA_HOME` and `XDG_CONFIG_HOME` are used, and failing those, `~/.local/share/rastro` and `~/.config/rastro`.

**Sync through the plugin host**
- **S4** WHEN `rastro sync project` runs a connector that submits evidence and a complete coverage report, THEN the evidence is stored, coverage records "found" for the window, and the run log shows `ok` with the plugin version and the core's counts.
- **S4a** WHEN a complete sync submits no evidence, THEN coverage records "empty" for the window.
- **S5** WHEN a connector submits evidence without a location, span, quote or trust class, or whose quote's UTF-8 byte length differs from `end - start`, THEN the core rejects it and counts it as rejected.
- **S6** WHEN a connector submits a kind its grant doesn't permit, or sets a source or scope other than the bound one, THEN the core rejects it and counts it as rejected.
- **S7** WHEN the connector answers `initialize` with an unsupported major version or a different manifest id, THEN the run fails with `protocol` before any evidence is ingested.
- **S8** WHEN the connector exits before `run.finish`, THEN the run fails with `interrupted`, coverage records "couldn't look", the evidence already accepted stays, and `rastro sync` exits non-zero.
- **S9** WHEN the connector sends a malformed or oversized line, or is silent past the timeout, THEN the core kills it, the run fails with `protocol` or `timeout`, and no partial message is ingested.
- **S10** WHEN the core starts and finds a run that never finished, THEN it marks that run `interrupted`.
- **S11** WHEN the connector reports "couldn't look" with a typed reason (e.g. `rate_limited`, with a retry time), THEN coverage stores the reason and the window is never reported as empty or complete.

**GitHub connector**
- **S12** WHEN syncing a fixture repo, THEN exactly the issues and PRs the owner authored, is assigned to, was asked to review, or is @mentioned in become evidence, together with their comments. Items with none of those don't.
- **S13** WHEN GitHub evidence is recorded, THEN each title, body and comment is its own observation with its URL, field path, byte span, quote, capture time and source update time. Prose is `untrusted content`; API fields are `structured`.
- **S14** WHEN results span several pages and a mid-run response is a rate limit, THEN the evidence from completed pages is kept and coverage records "couldn't look: rate_limited", not "found".
- **S15** WHEN GitHub content contains instruction-like or command-like text, THEN the connector's requests are still only the fixed GET endpoints. The fixture server records every method and path.
- **S16** WHEN the GitHub token is missing or rejected, THEN coverage records "couldn't look: auth" and no token appears in evidence, the run log or protocol messages.

**Re-sync**
- **S17** WHEN a source is synced again and nothing changed, THEN no new evidence is written, the run is linked to the existing evidence, and the unchanged count says so.
- **S18** WHEN a body changes A→B→A across three syncs, THEN there are three observations; `search` shows only the latest, and `search --history` shows all three.

**Search and coverage**
- **S19** WHEN `rastro search "x"` matches, THEN each result shows the quote, the link, the span, the capture time, the trust class, the scope, and the source's coverage and freshness.
- **S20** WHEN a search matches nothing, THEN the output says so and states the coverage of each source searched.
- **S21** WHEN a source's last complete check is older than its freshness rule, or its latest attempt failed, THEN `rastro coverage` answers "unknown", and still shows the last successful check and the failure reason.
- **S22** WHEN the full-text index is dropped and `rastro index rebuild` runs, THEN the same search returns the same results with their citations intact.
- **S23** WHEN any command runs with `--json`, THEN its output validates against the versioned operation schema.

**Owner sign-off**
- **S24** WHEN the owner registers one of their real repos and syncs, THEN searching for a phrase they remember returns it with a working link, and `rastro coverage` is accurate.

## Out of scope
- Claims, entities, extractors, `about` and `why` (slice 3); MCP (slice 2).
- More than one source kind; GitHub Discussions, reviews' inline comments, commits.
- Install-time permission UI (slice 15); scope grants for renderers and agents (slice 14).
- Scheduled triggers: sync is on demand only.
- Forgetting evidence (slice 14), export (slice 13).
- Cost metrics (slice 9); Windows.

## Risks
- **Security:** untrusted text can't choose what the connector calls; the token never enters stored data (S15, S16); the connector holds no action grant (ADR 0004). `gh` auth can write, so the connector uses only fixed GETs. No claim of OS sandboxing.
- **Performance:** GitHub search allows 30 requests a minute and caps results at 1,000. The four involvement queries are bounded by the window (default 90 days); hitting the cap marks coverage incomplete, not "found". FTS5 at this scale: n/a.
- **Migration:** a new database, so none yet. The schema is versioned from the first migration so later slices can migrate it; evidence is never rewritten.
- **Rollback:** delete the data directory and re-sync. Evidence is reconstructible from GitHub for this slice only (it isn't for sources that go away later).

## Seams
- **`rastro` CLI binary, end to end** with a temp `RASTRO_HOME`, `--json` output, and a test connector binary that can be scripted (`tests/fixtures/scripted-connector`): S1–S11, S17–S23.
- **GitHub connector against a local HTTP fixture server** (`GITHUB_API_URL` override), driven through the CLI: S12–S16.
- **Storage trait contract tests** run against the SQLite implementation: insert-only, transactions, rebuilding the index.

## Tickets
| # | What it delivers | Satisfies | Blocked by | Paths | Owner | Status |
|---|---|---|---|---|---|---|
| T1 | Workspace, CI, `rastro-core` config/location, SQLite store with migrations, `rastro source add/list`, `rastro coverage` "never looked", `--json` schema skeleton | S1–S3 | none | Cargo.toml, Cargo.lock, rust-toolchain.toml, .gitignore, .github/workflows/, crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/, schema/, tests/cli/ | agent | TO-DO |
| T2 | Plugin SDK + host: spawn, handshake, `evidence.submit`/`coverage.report`/`run.finish`, validation, run log, scripted test connector; basic `rastro search` with citations | S4, S4a, S5, S6, S19, S20 | T1 | crates/rastro-plugin-sdk/, crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/, schema/, tests/ | agent | TO-DO |
| T3 | Host hardening: version and manifest mismatch, crash, malformed or oversized lines, timeouts, interrupted-run recovery, typed "couldn't look" | S7–S11 | T2 | crates/rastro-core/src/host/, crates/rastro-core/src/runs/, tests/host/, tests/fixtures/scripted-connector/ | agent | TO-DO |
| T4 | GitHub connector: involvement queries, pagination, comment reads, span mapping, trust classes, auth and rate-limit failures, fixture server | S12–S16 | T2 | crates/rastro-connector-github/, tests/github/ | agent | TO-DO |
| T5 | Re-observation: unchanged linking, append on change, `run_evidence`, latest-per-location search, `--history` | S17, S18 | T2 | crates/rastro-core/src/evidence/, crates/rastro-storage-sqlite/, crates/rastro-cli/src/search*, tests/resync/ | agent | TO-DO |
| T6 | Freshness and "unknown", coverage display, `index rebuild`, `--json` schema validation across all commands | S21–S23 | T3, T5 | crates/rastro-core/src/coverage/, crates/rastro-storage-sqlite/, crates/rastro-cli/, schema/, tests/coverage/ | agent | TO-DO |
| T7 | Owner sync of a real repo and sign-off | S24 | T4, T6 | none | fbisca | TO-DO |
