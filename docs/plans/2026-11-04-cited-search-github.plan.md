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
| D19 | The core finds plugin manifests in `<config dir>/plugins/*/rastro-plugin.toml` and in the directories of `RASTRO_PLUGIN_PATH` (colon-separated, for development and tests); `command` resolves relative to the manifest's directory. A sync whose connector has no manifest fails as "couldn't look" | A built-in connector list | Plugins stay out of the core; tests point at the test connector | ADR 0004 |
| D20 | Migrations are named `YYYYMMDDHHmm_<name>.sql` (UTC) and tracked by name in a `schema_migrations` table, not by a sequence number | `0001.sql` numbering with `PRAGMA user_version` | Contributors on parallel branches don't collide or renumber | Owner, 2026-11-04; ADR 0002 |
| D21 | A connector process starts with a cleared environment: only `PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, plus the variables its manifest lists in `permissions.env` (e.g. `GH_TOKEN`) | Inheriting the owner's whole environment | Unrelated service tokens never reach a plugin | Codex review of the plugin host; ADR 0004 |
| D22 | S24 is satisfied by the owner's sign-off on 2026-11-04 (synced KaliosFit/app-flutter and FBisca/app-nextjs; cited hits with working links; coverage went from never looked to found), plus an ignored live test (`RASTRO_LIVE_REPO`, `RASTRO_LIVE_PHRASE`) as its re-runnable check | Leaving S24 untested | A real-repo scenario cannot run in CI, but it should stay re-runnable by the owner | Owner, 2026-11-04 |
| D23 | Coverage is answered per window: `rastro coverage` lists each window with its state, selection and retry time; a source's answer is "found" when any complete window found evidence, "empty" only when every complete window was empty, and freshness and failures apply as in S21 | Latest window as the whole answer | A quiet re-sync must not erase what an earlier window found | Gap review, glossary "Coverage" |
| D24 | Any rejected submission makes the run's window "couldn't look: protocol", over any reason the connector reports and over a failed finish; a failure the host itself observes (timeout, interrupted, storage, not installed) keeps its own reason; accepted evidence is still stored | Count only accepted items; a new `rejected` reason | An incomplete read never becomes "empty" or a complete "found" | knowledge §18 |
| D25 | `not_installed` joins the typed reasons; a manifest that fails its check records a failed run with "couldn't look: protocol" instead of aborting the sync | Map not-installed to `protocol`; abort on a bad manifest | Both are real, distinct outcomes the owner can act on | knowledge §18, D19 |
| D26 | Test-only overrides stay out of real use: the API base URL override is honoured only for loopback hosts; `RASTRO_NOW` and `RASTRO_HOST_*` only in debug builds | Keep them live in release; drop them | Tests need them; a stray value must not send the token elsewhere or make stale coverage look fresh | Gap review |
| D27 | `--keep-original` stores the full original the connector read (the item or comment as returned) beside each new observation; `rastro search --json` returns it; human output is unchanged | Store a separate copy file; show it in human output | Owner's choice per source, already promised by D13 | D13, knowledge §16 |

## Scenarios
**Registration and location**
- **S1** WHEN the owner runs `rastro source add project --connector github --repo o/p --scope personal`, THEN `rastro source list` shows `project` with its connector, selector, scope, freshness rule and window start, and the config file holds it.
- **S2** WHEN a source is registered but has never synced, THEN `rastro coverage project` reports "never looked".
- **S3** WHEN `RASTRO_HOME` is set, THEN data and config live under it. Otherwise `XDG_DATA_HOME` and `XDG_CONFIG_HOME` are used, and failing those, `~/.local/share/rastro` and `~/.config/rastro`.
- **S3a** WHEN anything tries to update or delete stored evidence, THEN the store rejects it and the evidence is unchanged.
- **S3b** WHEN migrations written on different branches are merged in any order, THEN each applies exactly once, in timestamp order, on new and existing databases, and a database with an unknown newer migration is refused.

**Sync through the plugin host**
- **S4** WHEN `rastro sync project` runs a connector that submits evidence and a complete coverage report, THEN the evidence is stored, coverage records "found" for the window, and the run log shows `ok` with the plugin version and the core's counts.
- **S4a** WHEN a complete sync submits no evidence, THEN coverage records "empty" for the window.
- **S5** WHEN a connector submits evidence without a location, span, quote or trust class, or whose quote's UTF-8 byte length differs from `end - start`, THEN the core rejects it and counts it as rejected.
- **S6** WHEN a connector submits a kind its grant doesn't permit, or sets a source or scope other than the bound one, THEN the core rejects it and counts it as rejected.
- **S6a** WHEN a source's scope is outside the scopes its connector's manifest declares, THEN sync refuses to run it and records "couldn't look"; and WHEN a connector's coverage report or evidence names a window other than the bound one, THEN the core rejects it.
- **S6b** WHEN a connector runs, THEN it sees only the allowed environment variables and those its manifest declares, never the owner's other secrets.
- **S6c** WHEN connector-supplied text reaches the terminal (rejection reasons, search results), THEN control characters are escaped.
- **S7** WHEN the connector answers `initialize` with an unsupported major version or a different manifest id, THEN the run fails with `protocol` before any evidence is ingested.
- **S7a** WHEN a manifest omits `permissions.env`, THEN it is treated as an empty list and the plugin still loads.
- **S8** WHEN the connector exits before `run.finish`, THEN the run fails with `interrupted`, coverage records "couldn't look", the evidence already accepted stays, and `rastro sync` exits non-zero.
- **S9** WHEN the connector sends a malformed or oversized line, or is silent past the timeout, THEN the core kills it, the run fails with `protocol` or `timeout`, and no partial message is ingested.
- **S9a** WHEN a connector writes to stderr, THEN the core relays each line with control characters escaped and length bounded, without ever blocking the connector.
- **S9b** WHEN a connector writes to stdout faster than the core reads, THEN the core holds a bounded number of lines in memory and the connector waits (backpressure); the run still completes.
- **S9c** WHEN a connector stops reading its stdin, THEN the core's write gives up at its deadline, kills the connector, and the run fails with `timeout`.
- **S9d** WHEN a connector sends `run.finish` but doesn't exit, or a child of it keeps its pipes open, THEN the core kills it after a shutdown deadline, keeps the finished run, and releases the sync lock.
- **S9e** WHEN a connector writes more stderr than the relay's bound, or the core's own stderr blocks, THEN the relay drops the excess and says how many lines it dropped, and neither the connector nor the run waits on it.
- **S9f** WHEN a connector sends invalid UTF-8, or a message that isn't a JSON-RPC 2.0 envelope (wrong `jsonrpc`, missing `method` or `id` where required), THEN the run fails with `protocol` and nothing from that line is ingested.
- **S9g** WHEN a connector causes many core diagnostics (rejected submissions or coverage reports) and the core's stderr isn't drained, THEN they go through the same bounded, lossy relay and the run never waits on them.
- **S9h** WHEN the core's reply to `run.finish` can't be written before its deadline, THEN the run fails with `timeout`.
- **S9i** WHEN a message has an `id` JSON-RPC doesn't allow (object, array, boolean), or mixes `method` with `result` or `error`, THEN the run fails with `protocol` and nothing from that line is ingested.
- **S9j** WHEN a connector, or a child of it, still holds its pipes when the run ends, THEN the host kills the connector's process group and releases its reader threads and pipe descriptors within the shutdown deadline.
- **S9k** WHEN the connector has exited, THEN the host signals its process group before reaping it, so the signal can never reach a process that reused its id.
- **S9l** WHEN storage rejects a submission (including span offsets the store can't represent, which are refused before storage) while the core's stderr is blocked, THEN the diagnostic goes through the bounded relay and the run doesn't wait on it.
- **S9m** WHEN one host process runs many syncs while its stderr is blocked, THEN the threads and queued output for printing stay bounded across runs, not per run.
- **S10** WHEN the core starts and finds a run that never finished, THEN it marks that run `interrupted`.
- **S10a** WHEN the core recovers a run that never finished, THEN coverage records "couldn't look: interrupted" for that run's source, scope and window, which are stored when the run starts.
- **S11** WHEN the connector reports "couldn't look" with a typed reason (e.g. `rate_limited`, with a retry time), THEN coverage stores the reason and the window is never reported as empty or complete.
- **S11a** WHEN a connector sends `run.finish` without a coverage report the core accepted (none sent, or every one rejected), THEN the run fails with `protocol` and coverage records "couldn't look: protocol".

**GitHub connector**
- **S12** WHEN syncing a fixture repo, THEN exactly the issues and PRs the owner authored, is assigned to, was asked to review, or is @mentioned in become evidence, together with their comments. Items with none of those don't.
- **S12a** WHEN an item in the window has comments updated before or after the window, THEN only the comments updated within `[from, to)` become evidence.
- **S12b** WHEN syncing, THEN the search queries the fixture server receives name the bound repo and window.
- **S13** WHEN GitHub evidence is recorded, THEN each title, body and comment is its own observation with its URL, field path, byte span, quote, capture time and source update time. Prose is `untrusted content`; API fields are `structured`.
- **S13a** WHEN labels or assignees are recorded, THEN each observation quotes the actual value at its own field path (`/labels/N/name`, `/assignees/N/login`), never a list the connector assembled.
- **S13b** WHEN prose contains multi-byte Unicode, THEN each span is in UTF-8 bytes and the quote equals the bytes at that span.
- **S14** WHEN results span several pages and a mid-run response is a rate limit, THEN the evidence from completed pages is kept and coverage records "couldn't look: rate_limited", not "found".
- **S14a** WHEN a search reports more than 1000 results or `incomplete_results`, THEN coverage records "couldn't look: result_cap", never "found".
- **S14b** WHEN GitHub answers 403 with `x-ratelimit-remaining: 0` or a secondary-limit signal (`retry-after`), THEN coverage records `rate_limited` with its retry time; any other 403 records `forbidden`.
- **S14c** WHEN an item's comments span several pages, THEN every page is read.
- **S15** WHEN GitHub content contains instruction-like or command-like text, THEN the connector's requests are still only the fixed GET endpoints. The fixture server records every method and path.
- **S15a** WHEN a `Link` header or a redirect points away from the API host, THEN the connector doesn't follow it, never sends the token there, and coverage records "couldn't look: protocol".
- **S16** WHEN the GitHub token is missing or rejected, THEN coverage records "couldn't look: auth" and no token appears in evidence, the run log or protocol messages.

**Re-sync**
- **S17** WHEN a source is synced again and nothing changed, THEN no new evidence is written, the run is linked to the existing evidence, and the unchanged count says so.
- **S18** WHEN a body changes A→B→A across three syncs, THEN there are three observations; `search` shows only the latest, and `search --history` shows all three.

**Search and coverage**
- **S19** WHEN `rastro search "x"` matches, THEN each result shows the quote, the link, the span, the capture time, the trust class, the scope, and the source's coverage and freshness.
- **S20** WHEN a search matches nothing, THEN the output says so and states the coverage of each source searched.
- **S21** WHEN a source's last complete check is older than its freshness rule, or its latest attempt failed, THEN `rastro coverage` answers "unknown", and still shows the last successful check and the failure reason.
- **S22** WHEN the full-text index is dropped and `rastro index rebuild` runs, THEN the same search returns the same results with their citations intact.
- **S22a** WHEN the index is rebuilt, THEN it behaves like a freshly migrated one: the same stemming and diacritics matches, and the same latest and history results.
- **S23** WHEN any command runs with `--json`, THEN its output validates against the versioned operation schema.
- **S23a** WHEN a known command runs with `--json` but its arguments don't parse, THEN it prints a schema-valid error result for that operation and exits non-zero; `--help` and `--version` still work.
- **S23b** WHEN a command gets an argument that isn't valid UTF-8, THEN it never panics: for a known operation under `--json` it prints that operation's schema-valid error result, and either way it exits 2.

**Owner sign-off**
- **S24** WHEN the owner registers one of their real repos and syncs, THEN searching for a phrase they remember returns it with a working link, and `rastro coverage` is accurate.

**Review of the whole slice**
- **S24a** WHEN the live check runs under `--ignored` with only one of its variables set, or one empty, THEN it fails naming what is missing; with neither set it is skipped.
- **S25** WHEN a source added with `--keep-original` syncs, THEN each new observation stores the full original beside its quoted span and `rastro search --json` returns it; a source without the flag stores none.
- **S25a** WHEN a connector sends an original that isn't a JSON object for a source that keeps originals, THEN the core rejects that submission, stores no original, and `rastro search --json` stays schema-valid.
- **S25b** WHEN a connector sends `"original": null` for a source that keeps originals, THEN the core rejects that submission like any other non-object original; an omitted original is still accepted.
- **S26** WHEN a source has more than one window, THEN `rastro coverage` lists each window `[from, to)` with its state, selection and, for "couldn't look", its retry time.
- **S26a** WHEN a later sync over a quiet period finds nothing new, THEN `rastro coverage` and each search hit still answer "found" for the source, and the quiet window shows "empty" in its own row.
- **S27** WHEN every submission in a run is rejected, THEN the window records "couldn't look: protocol", never "empty".
- **S27a** WHEN some submissions in a run are rejected, THEN the accepted evidence is stored and the window records "couldn't look: protocol", not "found".
- **S27b** WHEN a run has a rejected submission and also a connector-reported failure or a failed finish, THEN the window records "couldn't look: protocol" and the accepted evidence is kept.
- **S28** WHEN a source's connector isn't installed, THEN coverage records "couldn't look: not_installed", and `not_installed` is one of the published typed reasons.
- **S29** WHEN a source's manifest fails its check, THEN a failed run with "couldn't look: protocol" is recorded and any other source in the same sync still runs.
- **S30** WHEN the GitHub API base URL override names a host that isn't loopback, THEN the connector sends no request and coverage records "couldn't look: protocol".
- **S31** WHEN a release build runs with `RASTRO_NOW` or a `RASTRO_HOST_*` override set, THEN it ignores them.

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
- **`rastro` CLI binary, end to end** with a temp `RASTRO_HOME`, `--json` output, and a test connector binary that can be scripted (`crates/rastro-test-connector`, a workspace crate with `publish = false`): S1–S11, S17–S23.
- **GitHub connector against a local HTTP fixture server** (`GITHUB_API_URL` override), driven through the CLI: S12–S16.
- **Storage trait contract tests** run against the SQLite implementation: insert-only, transactions, rebuilding the index.

## Tickets
| # | What it delivers | Satisfies | Blocked by | Paths | Owner | Status |
|---|---|---|---|---|---|---|
| T1 | Workspace, CI, `rastro-core` config/location, SQLite store with migrations, `rastro source add/list`, `rastro coverage` "never looked", `--json` schema skeleton | S1–S3a | none | Cargo.toml, Cargo.lock, rust-toolchain.toml, .gitignore, .github/workflows/, crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/, schema/, tests/cli/ | agent | COMPLETE (#2, #3: converge pass, Codex review approve, CI green) |
| T2 | Plugin SDK + host: spawn, handshake, `evidence.submit`/`coverage.report`/`run.finish`, validation, run log, scripted test connector; basic `rastro search` with citations | S3b, S4, S4a, S5, S6, S6a–S6c, S19, S20 | T1 | Cargo.toml, Cargo.lock, docs/adr/0002-sqlite-fts5-storage.md, docs/adr/0004-plugin-manifest-and-grants.md, crates/rastro-plugin-sdk/, crates/rastro-test-connector/, crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/, schema/, tests/ | agent | COMPLETE (#4, #5: converge pass, CI green; Codex's last finding, connector stderr, moved to T3 as S9a) |
| T3 | Host hardening: version and manifest mismatch, crash, malformed or oversized lines, timeouts, interrupted-run recovery, typed "couldn't look", escaped connector stderr, optional `permissions.env` | S7, S7a, S8, S9, S9a, S10, S11 | T2 | crates/rastro-core/src/host/, crates/rastro-core/src/runs/, crates/rastro-core/src/lib.rs, crates/rastro-storage-sqlite/, crates/rastro-plugin-sdk/, crates/rastro-test-connector/, crates/rastro-cli/tests/host.rs | agent | COMPLETE (#6) |
| T4 | GitHub connector: involvement queries, pagination, comment reads, span mapping, trust classes, auth and rate-limit failures, fixture server | S12–S16 | T2 | Cargo.toml, Cargo.lock, crates/rastro-connector-github/, crates/rastro-plugin-sdk/src/lib.rs, docs/decisions/knowledge.md | agent | COMPLETE (#7: converge pass, CI green; Codex review findings moved to T9) |
| T5 | Re-observation: unchanged linking, append on change, `run_evidence`, latest-per-location search, `--history` | S17, S18 | T3, T8, T10, T12 | crates/rastro-core/src/evidence/, crates/rastro-core/src/host/, crates/rastro-core/src/runs/, crates/rastro-core/src/lib.rs, crates/rastro-storage-sqlite/, crates/rastro-cli/src/, crates/rastro-cli/tests/resync.rs, crates/rastro-cli/tests/host.rs, schema/ | agent | COMPLETE (#13: converge pass, Codex approve, CI green) |
| T6 | Freshness and "unknown", coverage display, `index rebuild`, `--json` schema validation across all commands | S21–S23 | T3, T5 | crates/rastro-core/src/coverage/, crates/rastro-core/src/lib.rs, crates/rastro-core/src/runs/, crates/rastro-storage-sqlite/, crates/rastro-cli/, crates/rastro-connector-github/tests/, schema/ | agent | COMPLETE (#14: converge pass, CI green; Codex review findings moved to T13) |
| T7 | Owner sync of a real repo and sign-off | S24 | T4, T6, T9, T13, T14, T15, T16 | none | fbisca | COMPLETE (owner sign-off 2026-11-04) |
| T8 | Host review fixes: bounded stdout with backpressure, write and shutdown deadlines, bounded lossy stderr relay, strict UTF-8 and JSON-RPC envelope, recovered runs get "couldn't look", finish without coverage fails | S9b–S9f, S10a, S11a | T3 | crates/rastro-core/src/host/, crates/rastro-core/src/runs/, crates/rastro-core/src/lib.rs, crates/rastro-storage-sqlite/, crates/rastro-plugin-sdk/, crates/rastro-test-connector/, crates/rastro-cli/tests/host.rs, crates/rastro-cli/tests/sync_search.rs | agent | COMPLETE (#8: converge pass, CI green; Codex re-review findings moved to T10) |
| T9 | GitHub review fixes: comments filtered to the window, query asserted, real label and assignee values, Unicode spans, result cap, 403 classification, comment pagination, off-host links and redirects refused | S12a, S12b, S13a, S13b, S14a–S14c, S15a | T4 | crates/rastro-connector-github/ | agent | COMPLETE (#9: converge pass, Codex re-review approve, CI green) |
| T10 | Host review fixes, round 2: core diagnostics through the bounded relay, `run.finish` reply deadline, JSON-RPC id and shape validation, process-group cleanup that releases pipes and threads; tighter S9b, S9d and S9f tests | S9g–S9j | T8 | crates/rastro-core/src/host/, crates/rastro-core/src/runs/, crates/rastro-core/src/lib.rs, crates/rastro-plugin-sdk/, crates/rastro-test-connector/, crates/rastro-cli/tests/host.rs, crates/rastro-cli/tests/sync_search.rs | agent | COMPLETE (#10: converge pass, CI green; Codex re-review findings moved to T12) |
| T11 | GitHub test tightening: comment window at its exact `from` and `to`, decoded search `q` asserted exactly | S12a, S12b | T9 | crates/rastro-connector-github/tests/ | agent | COMPLETE (#11: converge pass, Codex approve, CI green) |
| T12 | Host review fixes, round 3: signal the process group before reaping, storage-error diagnostics through the relay with span offsets range-checked, one host-wide bounded printer; zombie-aware holder check in tests | S9k–S9m | T10 | crates/rastro-core/src/host/, crates/rastro-core/src/evidence/, crates/rastro-core/src/lib.rs, crates/rastro-test-connector/, crates/rastro-cli/tests/host.rs | agent | COMPLETE (#12: converge pass, Codex re-review approve, CI green) |
| T13 | Review fixes for freshness and output: argument errors under `--json` print the operation's schema error envelope; rebuilt-index parity test | S22a, S23a | T6 | crates/rastro-cli/, crates/rastro-storage-sqlite/, schema/ | agent | COMPLETE (#15: converge pass, CI green; Codex re-review finding moved to T14) |
| T14 | CLI arguments read as OS strings, so non-UTF-8 input gets the usage error, never a panic | S23b | T13 | crates/rastro-cli/ | agent | COMPLETE (#16: converge pass, Codex re-review approve, CI green) |
| T15 | Tests hold under load: deadlines only as tight as the scenario being tested, no flakes with several suites running at once | S9, S15a | T14 | crates/rastro-cli/tests/, crates/rastro-connector-github/tests/ | agent | COMPLETE (#17: converge pass, CI green; Codex review finding moved to T16) |
| T16 | Each test process runs its own snapshot of the binaries, never a copy cached by size and time | S9, S15a | T15 | crates/rastro-cli/tests/, crates/rastro-connector-github/tests/ | agent | COMPLETE (#18) |
| T17 | Keep the full original when the source opts in | S25 | none | crates/rastro-plugin-sdk/, crates/rastro-connector-github/, crates/rastro-storage-sqlite/, crates/rastro-core/, crates/rastro-cli/, schema/ | agent | COMPLETE (#22) |
| T18 | Coverage per window; rejected submissions never make a window found or empty | S26, S26a, S27, S27a | none | crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/, schema/ | agent | COMPLETE (#23) |
| T19 | A failed manifest check is a failed run, not an aborted sync; `not_installed` is a typed reason | S28, S29 | T18 | crates/rastro-core/, crates/rastro-plugin-sdk/, crates/rastro-cli/, docs/decisions/knowledge.md | agent | TO-DO |
| T20 | Test-only overrides stay out of real use: loopback-only API override, debug-only clock and host limits | S30, S31 | T17, T19 | crates/rastro-core/, crates/rastro-connector-github/, crates/rastro-cli/ | agent | TO-DO |
| T21 | The live check fails loudly when half-configured | S24a | none | crates/rastro-cli/tests/live.rs | agent | COMPLETE (#21) |
| T22 | Review fix: an original must be a JSON object, or the submission is rejected | S25a | T17 | crates/rastro-core/, crates/rastro-cli/tests/, crates/rastro-test-connector/ | agent | COMPLETE (#24) |
| T23 | Review fix: a rejected submission makes the window "couldn't look: protocol" whatever else failed | S27b | T18 | crates/rastro-core/, crates/rastro-cli/tests/, crates/rastro-test-connector/ | agent | COMPLETE (#26) |
| T24 | Review fix: an explicit null original is rejected for a source that keeps originals | S25b | T22 | crates/rastro-plugin-sdk/, crates/rastro-core/, crates/rastro-cli/tests/, crates/rastro-test-connector/ | agent | COMPLETE (#25) |
| T25 | Review fix: the rejection-precedence test compares instants, not timestamp strings | S27b | T23 | crates/rastro-cli/tests/ | agent | TO-DO |
