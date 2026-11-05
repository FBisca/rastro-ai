# Plan: Claims hardening

Slug: `claims-hardening`. Living spec: `docs/specs/claims.md` (amended; `docs/specs/surfaces.md` amended for R1, R27, R28). Status: APPROVED 2026-11-05.

## Why now
The gap review of the archived plan `2026-11-04-claims-and-about.plan.md` found behaviour that contradicts or only partly meets its decisions:
- old logins stop resolving;
- a hidden scope can make a visible alias unresolvable;
- `claims rebuild` wipes the output of a pair it can't replay. On the owner's database, a missing extractor plugin would wipe all 2248 claims.
- type conflicts are checked per location only;
- malformed entity forms return empty instead of an error;
- a dead T3 `why` path remains;
- an unrecorded additive change to `sync`'s result.

The owner accepted every recommendation (2026-11-05).

Out of this plan, and planned later together with `source rename/remove` (open question #9): the extractor binding in source config, and unbind.

## Sources
| Source | What it settles |
|---|---|
| Owner, 2026-11-05 session: gap-review items 1–5, 7, 8 accepted; consolidated grilling (21 questions) accepted in full | Every decision below |
| Independent review of the grilling (Claude Opus; Codex was over its usage limit) | Amendments folded into D1–D21; fact corrections; missing decisions D15, D8, D21 |
| Gather report and real DB (immutable read), 2026-11-05 | 719 entities, 2248 claims, 4480 justifications; 0 links; 3 aliases, all observed; 0 renames; 0 reused logins; 5 entities typed at more than one location (442/259/13/258/148), 0 with more than one type; `kalios`/work unregistered with 1304 justifications; `kalios-mobile`/personal registered, never synced, same repo as `kalios` |
| `docs/decisions/knowledge.md` §5, §14, §24, §26, §29, §37–§44, §48, §49 | Statuses; scopes; one identifier is one entity; supersession and conflict; extraction states; justifications; retirement; visible-only scoped answers; entry conditions; accepted cold read |
| `docs/decisions/vocabulary.md` §13, §15–§17 | Cardinality per observation context; type and alias are claims; an old login stays a historical candidate; a reused login is ambiguous; subtyping decides type conflict; fixed checks |
| `docs/decisions/surfaces.md` §18, §22–§31 | Error model; existing contracts don't change (amended here); `about` forms; extraction state shown; scope filters every derived field; canonical alias form |
| `docs/decisions/plugins.md` §5, §15, §19 | The owner can revoke a permission; one binding per source; retirement |
| ADR 0008 (+2026-11-05 amendment, build notes), ADR 0009 (+amendment, correction, build notes, benchmark note) | Core validation, type grounding, type scalarity; projection tables, slot, rebuild by shadow swap, scope mechanism |
| `docs/specs/claims.md` R5, R6, R14, R17, R22, R29–R31; `docs/specs/surfaces.md` R1, R27, R28 | Requirements this plan rewords or extends |
| Code at `8317921` | `ALIAS_SUBJECTS` (observed only) and `aliases()` in `crates/rastro-storage-sqlite/src/knowledge.rs`; scoped roots in `scoped.rs`; `rebuild_claims` in `crates/rastro-core/src/runs/mod.rs` (skips, then publishes); type conflict per (location, field) in `lifecycle.rs`; `validate_entity` in `crates/rastro-core/src/knowledge.rs`; T3 `why` in core and storage `lib.rs`; `RunReport.extraction` |

## Decisions
| # | Choice | Rejected | Why | Source |
|---|---|---|---|---|
| D1 | Alias lookup counts every status except `hypothesis` and `retracted`: today `observed` and `superseded`; `confirmed` from slice 7 | Observed only; any status | Superseded aliases are the historical candidates of vocabulary §15; `confirmed` is a status (knowledge §5) | Owner; review |
| D2 | A login held by several entities gives the ambiguous-alias operation error, listing every candidate marked `current` (observed or confirmed) or `historical` (superseded). For a scoped read, the marks come from visible justifications in the replay, never from stored status. One holder resolves directly | Prefer the current holder; unmarked list | Vocabulary §15: a reused login is ambiguous and never merges; stored status is full-scope (knowledge §44) | Owner; review |
| D3 | After resolving through an old login, `about` shows the current aliases only. `why` on the alias claim shows its history. No contract change | Add historical aliases or `resolved_by` | Identity plus current aliases answer the question; revisit with open question #10 | Owner |
| D4 | Scoped alias roots are the matching alias claims with at least one fully visible justification. No stored status is used in root selection; D1's status set, ambiguity and marks are applied inside the replay | Only drop the status filter | Hidden roots feed the work budget, and a hidden supersession drops a visible root: both break surfaces R28 and knowledge §44 | Owner; review |
| D5 | `claims rebuild` carries over the output of a pair whose extractor never started: not installed, failed its check, or scope forbidden. An extractor that starts and fails still aborts the whole rebuild (R29). The carried rows are justifications, spans, claims, entities and the checkpoint. Statuses, occurrences and links are recomputed in full for every touched component | Fail the whole rebuild; today's wipe; carry occurrences and links | Parity with sync and knowledge §40 (last good output); lifecycle stays f(evidence, justifications); 4 real claims are shared across pairs | Owner; review |
| D6 | Rebuild's exit code follows sync's: non-zero when any pair reads `failed` (check failure, forbidden), 0 when only `not_installed` | Always 0 | Sync fails on `failed` (`Response::succeeded`) | Review |
| D7 | `claims.rebuild`'s result gains a `pairs` list (source, scope, run id, extractor, version, state, reason, counts), additive within v2. The CLI prints a line per carried-over pair | Log only; nothing | The owner must see what wasn't re-derived (surfaces §25) | Owner |
| D8 | A forbidden pair (its scope is outside the extractor manifest's `permissions.scopes`) is carried over like `not_installed` and reads `failed` with reason `forbidden`; no new state. Entry condition in knowledge §48 for slice 14/15: does revoking a scope withdraw output already derived under it? | Drop its output; a new `forbidden` state | The plugin sees nothing new and scopes still filter reads (§14); the state list stays (§29); revocation (plugins §5) is undecided | Owner; review |
| D9 | Carried output isn't precious and is re-derived once the extractor returns (ADR 0009 note). R22 "rebuild equals incremental" is stated for when every pair replays; carry-over gets its own requirement | Treat carried rows as precious | Knowledge §12: only evidence, confirmations, rules and outcomes are precious | Review |
| D10 | Only `rastro:type` gets an entity-wide conflict check. Aliases and other predicates stay per (location, field) | Every scalar predicate | An old login not yet re-read must not conflict with the new one; other GitHub scalars have the item location as subject | Owner |
| D11 | Only `conflicts_with` between `rastro:type` claims is entity-wide, computed in the lifecycle rollup between claims live anywhere in the component, unless one type is an ancestor of the other. Dominance, `supersedes` links and incremental slot selection stay per (location, field) | Entity-wide dominance and supersession by `source_updated_at` | Item-level time isn't comparable across items (§26: incomparable means conflict); an entity-wide group would replay a hub's 442 (benchmark 20k) type derivations on every observation | Owner; review |
| D12 | The displayed type is the most specific live type when the types are compatible. When they conflict, it's the type with the newest known time, ties broken by claim id. One function, computed in the replay for scoped reads | A list of types; `ORDER BY id` | No contract change; deterministic; the conflict stays visible on the type claims | Owner; review |
| D13 | Entity forms, syntax only. No whitespace in the `e_` id, the URL, the namespace or the kind; a `<namespace>:<kind>/<value>` value may contain inner whitespace but not at either end (D23): `e_` + 32 lowercase hex; an exact URL with an RFC 3986 scheme (`<scheme>://<non-empty>`); `<namespace>:<kind>/<value>` with namespace and kind non-empty, kind without `/`, and value non-empty (it may contain `/`, `[`, `]`). Anything else is an operation error naming the three forms; never a JSON-Schema `pattern`. A well-formed form that matches nothing gives the empty answer with coverage | Checking namespaces against plugins | The core can't name `github` (AGENTS.md); surfaces §18 and R30 keep it an operation error; fits every real identifier and alias (fragments, `[bot]`) | Owner; review |
| D14 | `claim.submit` rejects a subject or object identifier outside D13's grammar, using the same function as the read side; counted rejected | No submit check | Otherwise an extractor can mint an entity `about` can't reach; real data unaffected | Owner; review |
| D15 | Vocabulary load rejects an alias kind containing `/`, `:` or whitespace; submit rejects an alias value that is empty, whitespace-only, or has whitespace at either end; inner whitespace is allowed (D23) | No check | Same reachability argument as D14 | Review |
| D16 | Lookup doesn't check alias kinds against installed vocabularies | Check at lookup | Submit already checks kinds (`claims.rs`); a lookup check would break stored aliases after an extractor is removed | Owner |
| D17 | Remove the T3 `why` path (`Storage::why`, `Rastro::why`, `View::why`, `SupportView`, `Why`) and `Rastro::extraction_states`. The CLI test helper moves to the read-only `Reader::extraction_states`, keeping its setup (plugins symlink, a writing open where a test relies on it). Tests needing every support call storage `Snapshot::explain` uncapped | Keep a test-only helper; the catalog `why` (capped at 50) | One path each; nothing unbounded or scope-blind remains | Owner; review |
| D18 | A new `surfaces.md` entry amends §22: `sync`'s result carries an optional `extraction` object (run id, extractor, version, state, reason, evidence and claim counts), absent when no extractor is bound; `claims.rebuild` gains `pairs` (D7). Both are additive within v2. Surfaces R1 is reworded: existing operations are unchanged except additive changes recorded in `surfaces.md`. The archived plan isn't edited; the entry references its D18 | Edit the archived plan; leave it unrecorded | AGENTS.md: decisions live in decision files | Owner; review |
| D19 | Tickets: T1 docs; T2 remove the T3 `why` path; T3 aliases (D1–D4); T4 rebuild (D5–D9); T5 types (D10–D12), after T3 and T4; T6 entity forms (D13–D15), after T3. T3 and T4 run in parallel after T2. Odd tickets are built by Claude, even tickets by Codex when available, else Claude. During the GitHub outage CI isn't waited on: merge once the local gate passes (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`), with converge and a code review | Parallel T3–T6 | Paths overlap: about_why.rs, mcp.rs, storage knowledge.rs, lifecycle.rs | Owner; review |
| D20 | Amend `docs/specs/claims.md`, and `surfaces.md` R1, R27, R28. No new spec. No owner sign-off ticket: no new user surface. Tests use `fn S<n>_…` while the plan runs and are retagged at archive | A new spec | Same area | Owner |
| D21 | Scenarios cover one location observed in two scopes (as `kalios-mobile` next to `kalios` will be): alias marks, roots and type conflicts come only from the visible scope's justifications | Single-scope tests only | Lifecycle groups (location, field) across scopes | Review |
| D22 | T4's Paths add `crates/rastro-core/src/host/extract.rs`: the extraction report records whether the extractor started (past its check and scope), so rebuild can tell "never started" (carry over) from "started and failed" (abort) | Infer it from the reason string | Both fail with reason `protocol` today | Lead; owner confirmed 2026-11-05 |
| D23 | Alias values may hold inner whitespace (`slack:name/Felippe Bisca`): D13 and D15 are amended so the value of `<namespace>:<kind>/<value>` allows inner whitespace, never leading or trailing; namespace and kind allow none. `github:login/a b` is well-formed and gives the empty answer. T6 also amends `surfaces.md` §34, `vocabulary.md` §23 and the ADR 0008 amendment, and drops their "awaiting the owner's confirmation" marker | No whitespace at all (D13 as first written); percent-encoding | `rastro:alias` is core, not GitHub: names from later sources (Slack, mail display names, people) contain spaces. The CLI quotes the form; MCP passes JSON | Owner, 2026-11-05 |

## Scenarios
**Aliases**
- **S1** WHEN an account's login changed and a re-read superseded the old login's alias claim, THEN `about github:login/<old>` returns that account's claims, as `about github:account/<id>` does.
- **S2** WHEN one login is an alias of two accounts (current on one, historical on the other), THEN `about` on it returns the ambiguous-alias operation error listing both candidates, each marked `current` or `historical`.
- **S3** WHEN `about` resolves through an old login, THEN its subject aliases list only current aliases, and `why` on the old alias claim shows its known intervals.
- **S4** WHEN an alias claim with status `hypothesis` or `retracted` matches (set up in storage), THEN it isn't a lookup candidate.
- **S5** WHEN a visible alias claim at a location is superseded only by a hidden-scope observation at that same location, THEN a scoped `about` on that alias resolves exactly as a projection rebuilt from the visible scopes' evidence does, and any candidate marks come from visible justifications.
- **S6** WHEN hidden alias claims match the queried alias and exceed a small explicit work budget, THEN the scoped answer equals the unknown-entity answer.
- **S7** WHEN the random scope-set oracle runs, THEN it also queries by alias, and each scoped answer equals the per-scope rebuild.
- **S8** WHEN one location is observed in two scopes with different logins, THEN a scoped read's alias roots, resolution and marks come only from the visible scope's justifications.

**Rebuild**
- **S9** WHEN `claims rebuild` runs and a pair's bound extractor isn't installed, THEN:
  - that pair's justifications, spans, claims, entities and checkpoint are carried over;
  - every touched component's statuses, occurrences and links are recomputed;
  - the other pairs are rebuilt;
  - the command exits 0, and its result lists the pair with state `not_installed`.
- **S10** WHEN a pair's extractor fails its check, or the pair's scope isn't in the extractor manifest's `permissions.scopes`, THEN its output is carried over as in S9, the pair reads `failed` with its reason (`forbidden` for the scope case), and the command exits non-zero.
- **S11** WHEN an extractor starts during a rebuild and then fails, THEN the rebuild aborts and the previous projection stays readable and unchanged.
- **S12** WHEN a claim is justified by both a carried-over pair and a rebuilt pair, THEN its status, occurrences, links and revision equal those of the incremental projection.
- **S13** WHEN `claims rebuild` completes, THEN:
  - its result has a `pairs` list with source, scope, run id, extractor, version, state, reason and counts;
  - the CLI prints a line per carried-over pair;
  - the schema change is additive within v2, and the drift test accepts it as recorded.
- **S14** WHEN `claims rebuild` runs on a copy of the owner's real database with the extractor plugin hidden, THEN all of the copy's claims remain. This is an ignored test, given the copy's path.

**Types**
- **S15** WHEN an entity has live `rastro:type` claims of unrelated types at different locations, THEN both carry `conflicts_with` and neither supersedes the other.
- **S16** WHEN the types at different locations are a type and one of its supertypes, THEN there's no conflict, and `about` and `why` display the most specific type.
- **S17** WHEN an entity's types conflict, THEN `about` and `why` display the type with the newest known time, ties broken by claim id. A scoped read computes it from visible justifications only.
- **S18** WHEN the same location's type changes (Bot to User), THEN the new type claim supersedes the old one, as before.
- **S19** WHEN a new item observation arrives for an entity typed at many locations, THEN only the affected (location, field) slots replay: the hub scaling bound still holds, and a rebuild equals the incremental projection.
- **S20** WHEN one location is observed in two scopes with different types, THEN a scoped read's type conflicts and displayed type come only from the visible scope's justifications.

**Entity forms**
- **S21** WHEN `about` gets `foo:bar`, `github:FBisca`, `github:login/`, `:x/y`, an `e_` id with uppercase hex, or a form with whitespace in its namespace or kind or at either end of its value, THEN it returns an operation error naming the three forms. On MCP it's a result with `isError: true`, not a JSON-RPC error.
- **S22** WHEN `about` gets a well-formed form that matches nothing (`gitlab:login/x`, `github:login/FBisca/x`, `github:login/a b`), THEN it returns the empty result with coverage.
- **S23** WHEN `about` gets a canonical URL with a fragment, `github:account/<id>` or `github:login/vercel[bot]`, THEN each is accepted.
- **S24** WHEN an extractor submits a claim whose subject or object identifier is outside the accepted forms, THEN it's rejected and counted.
- **S25** WHEN a vocabulary declares an alias kind containing `/`, `:` or whitespace, THEN loading it is refused; WHEN a submitted alias claim's value is empty, whitespace-only, or has whitespace at either end, THEN it's rejected and counted; WHEN it has inner whitespace (`Felippe Bisca`), THEN it's accepted and `about` resolves it.

## Out of scope
- An extractor binding in source config, and unbind; `source rename/remove` (open question #9, a later plan).
- A "not resolved" hint on `about` (open question #10).
- Withdrawing derived output when a scope is revoked (entry condition, D8).
- Faster cold scoped reads (knowledge §49).
- Owner intents, merges, Persons and hypotheses (later slices).

## Risks
- **Security:**
  - Scopes: alias roots, marks and displayed types come from visible justifications only (S5–S8, S17, S20). Hidden work never shows in a scoped answer (S6).
  - Submit: the identifier and alias grammar is checked (S24, S25).
  - Carried-over output stays scope-filtered on every read (D8).
- **Performance:**
  - Entity-wide type conflicts are computed in the rollup over the component already loaded, so slot selection doesn't change (S19).
  - Historical alias lookup adds statuses to an indexed query.
  - Rebuild carry-over copies rows only for pairs it skips.
- **Migration:** no schema migration expected. The lifecycle rollup changes, so a `claims rebuild` (or the next sync) recomputes the links. The `claims.rebuild` result schema change is additive.
- **Rollback:** revert the PRs. The projection is disposable, so `claims rebuild` on the old binary restores it. Carried-over rows are re-derived once the extractor is back.

## Seams
- **`rastro` CLI with a temp `RASTRO_HOME`**, the scripted connector and extractor, and `--json` against the v2 schema (`crates/rastro-cli/tests/claims.rs`, `about_why.rs`): S1–S4, S9–S13, S15–S18, S21–S25.
- **Storage-level tests** (`crates/rastro-storage-sqlite/tests/`): S4, S6, S12, S19, and lifecycle unit cases.
- **`rastro mcp --scope` with the scripted client** (`crates/rastro-cli/tests/mcp.rs`): S5, S7, S8, S20, and the MCP half of S21.
- **Ignored test on a copy of the real database:** S14.

## Tickets
| # | What it delivers | Satisfies | Blocked by | Paths | Owner | Status |
|---|---|---|---|---|---|---|
| T1 | Decision entries: knowledge.md (D5, D8 entry condition in §48, D9), vocabulary.md (D1–D2, D10–D11, D15), surfaces.md (D3, D7, D13, D18; amend §22, §31). ADR 0008 amendment (D14, D15). ADR 0009 amendment (D5, D9, D11, the scope-mechanism note for D4) | none (docs) | none | docs/adr/, docs/decisions/, docs/open-questions.md | agent | COMPLETE (#76 merged; converge PASS, review fixed, CI green) |
| T2 | Remove the T3 `why` path and `Rastro::extraction_states` (D17); retarget the lifecycle and hub tests and the CLI test helper | none (refactor; existing tests stay green) | none | crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/tests/ | agent | COMPLETE (#77 merged; converge PASS, review fixed, CI green) |
| T3 | Historical alias lookup, marked ambiguity and visible-only scoped alias roots (D1–D4) | S1, S2, S3, S4, S5, S6, S7, S8 | T1, T2 | crates/rastro-storage-sqlite/, crates/rastro-core/src/knowledge.rs, crates/rastro-cli/tests/, schema/, crates/rastro-operations/ | agent | COMPLETE (#78 merged; converge PASS, review blockers fixed, CI green) |
| T4 | Rebuild carries over unreplayable pairs, sets the exit code and reports `pairs` (D5–D9); records the `sync.extraction` amendment in tests | S9, S10, S11, S12, S13, S14 | T1, T2 | crates/rastro-core/src/runs/, crates/rastro-core/src/lib.rs, crates/rastro-core/src/host/extract.rs, crates/rastro-storage-sqlite/, crates/rastro-operations/, crates/rastro-cli/, schema/ | agent | COMPLETE (#79 merged; converge PASS, review blockers fixed, CI green) |
| T5 | Entity-wide type conflict and the displayed type (D10–D12) | S15, S16, S17, S18, S19, S20 | T3, T4 | crates/rastro-storage-sqlite/, crates/rastro-core/src/claims.rs, crates/rastro-cli/tests/ | agent | TO-DO |
| T6 | Entity form grammar on read and submit; alias kind and value checks (D13–D16, D23); amend the alias-value records (D23) | S21, S22, S23, S24, S25 | T3 | crates/rastro-core/, crates/rastro-plugin-sdk/, crates/rastro-operations/, crates/rastro-cli/tests/, schema/, docs/decisions/, docs/adr/ | agent | TO-DO |
