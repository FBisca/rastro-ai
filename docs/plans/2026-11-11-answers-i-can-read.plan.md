# Plan: Answers I can read

Slug: `answers-i-can-read`. Living spec: `docs/specs/claims.md` (labels, `tracker:state`); `docs/specs/evidence.md` (sync progress, rename warning). Status: APPROVED 2026-11-12.

## Why now
Slices 1–4 shipped in three days, but the owner can't use them yet:
- `about me` returns bare issue and pull-request identifiers, so an agent needs one call per entity to learn what is on the owner's plate.
- `rastro sync` prints nothing until every source finishes. On the owner's database the first read of one source was interrupted three times, 10–42 s in, before it completed.
- Renaming a source re-read its whole window (1,007 items) without a word.

This slice closes M1 (roadmap row 4b). **Switch day:** the morning after it lands, the owner preps standup with `about me` over MCP instead of opening Linear and GitHub.

Process rules from the retro (owner, 2026-11-11): build only what this outcome needs, ≤8 decisions, Codex review only for ADR-level choices (none here).

## Sources
| Source | What it settles |
|---|---|
| Owner, 2026-11-11 session: grilling Q1–Q11, all recommendations accepted | D1–D8 |
| `docs/roadmap.md` row 4b | Outcome and switch day |
| `docs/decisions/knowledge.md` §69 (on branch `docs/retro-roadmap`) | Label fields, 200-char bound with clipping flag, trust class, state only from an observed claim, label is evidence and not a claim, scoped reads use visible evidence only |
| `docs/decisions/knowledge.md` §22, §28, §37, §44, §45 | Sources keyed by name; `--reread` from `window_start`; ordering by `captured_at` then stored order; visible-only scoped answers; revision depends on visible state only |
| `docs/decisions/surfaces.md` §9, §12, §17, §19, §26, §33 | Every command has `--json`; UTF-8-safe excerpts with clipping flags; untrusted content labelled; MCP declares no `outputSchema`; a launch scope filters every derived field; additive changes within v2 are recorded |
| `docs/decisions/vocabulary.md` §11 | `code:state` is the only state predicate so far |
| `docs/specs/evidence.md` R17–R29, R73 | Diagnostics never block a run; re-read semantics |
| Issue #102 | Silent, slow sync; item 4 (why a source re-read everything) answered by the rename |
| Gather report and a copy of the live DB (read-only), 2026-11-11 | Counts below |

Counts from the live DB:
- `about me` covers 1,093 entities. 1,036 have a label source: 714 PRs, 321 Linear issues and 1 Linear user. 57 have none: 55 comments, 1 GitHub account and the Person.
- 0 of 3 repositories have name evidence.
- 0 of 338 Linear issues have a state claim, although all have `/state/name` evidence.
- Titles: the longest is 132 chars and 132 bytes. 55 contain multibyte chars, 0 have changed text over time, 0 contain control chars.
- 148 title locations are stored under two scopes, from the `kalios` → `kalios-mobile` rename.
- First GitHub reads take 54 s to 2 min 49 s; incremental reads take 1–4 s.

## Decisions
| # | Choice | Rejected | Why | Source |
|---|---|---|---|---|
| D1 | Labels sit next to the aliases, as `subject_label` / `object_label` on each claim in `about` and `why`, with fields `{text, url, evidence_id, field, captured_at, trust_class, clipped, state}`. Additive within v2, recorded in surfaces. The MCP tool descriptions say label text is untrusted data. CLI human output shows the label after the id, through `display_safe` | A label nested inside an entity object; a new result section | Follows `subject_aliases` / `object_aliases`; keeps v2 additive | Owner Q1; surfaces §17, §19, §33 |
| D2 | A label comes only from evidence at the entity's own location: `/title` for GitHub issues and PRs and for Linear issues, `/name` for Linear users. No label for repositories, comments, GitHub accounts or Persons. §69 is amended: no repository label, and no Person label borrowed from a linked account | Deriving a repo name from its URL (a parse); a Person label from an account (a derivation close to identity); Linear `/displayName` | No evidence exists for the excluded kinds; a borrowed label would break "evidence, not a claim" | Owner Q2; gather counts |
| D3 | Label rules settled by §69: at most 200 chars, cut on a UTF-8 boundary, `clipped` flag; latest visible observation by `captured_at` desc, then evidence id desc; `state` is the value of the current observed state claim, or null; a label never touches claims, supports, alias lookup or the revision | Byte bound (surfaces §12 uses bytes; §69 says characters) | §69 is the newer, specific decision; no live title reaches either bound | knowledge §37, §44, §45, §69 |
| D4 | Add `tracker:state` from Linear's `/state/name` (structured): a vocabulary entry and a Linear extractor rule, with the extractor version bumped | Leaving Linear issues without state; parsing state into the label | Without it, 321 of the 1,036 labelled entities show no Done or In Progress, and standup prep fails | Owner Q3; vocabulary §11 |
| D5 | Leave out the Linear identifier (`ENG-123`). A Linear label's `url` is null, because its location is `linear:issue/<uuid>` | A second piece of evidence per label; a new identifier claim | Not needed for the switch day; revisit if standup prep shows it is | Owner Q4 |
| D6 | Sync progress goes to stderr only; stdout and `--json` don't change. Per source: a start line with its reason (`first read from <t>`, `re-read from <t>`, `incremental since <t>`), a line per stage (read, extract, project), with extract saying when it replays stored evidence for a new extractor version, a running item count at most every 2 s, and a result line that includes `rate-limited, retry at <t>` from the stored outcome. An observer seam in `runs::sync_with_reread` / `host::run`; a failing stderr never blocks or fails a run | Progress in `--json`; a TTY-only spinner; a new connector message for rate-limit waits | The connector never waits on a rate limit (it fails as `rate_limited`, and coverage records the gap) | Owner Q5, Q9; evidence R21, R23 |
| D7 | Rename warning, before the read: a configured source with no complete check, plus a stored source no longer in config with the same connector and selector (whatever its scope), gives a stderr line and an entry in an additive `warnings` field of the sync result. Warn only: the read still runs | Matching scope too (it would have missed the real rename, work → personal); a prompt or refusal; a stable source id | Sync has no prompt and `--json` must stay non-interactive; a stable id changes §22 and needs an ADR | Owner Q6; knowledge §22 |
| D8 | Out of scope: removing the duplicated `kalios` evidence (open question #9), a stable source id, labels from scope-hidden evidence, the slow half of #102 (timings in `--json`, a persistent profile cache, background replay). #102 stays open; the PRs reference it, and a comment lists what shipped when the slice lands | Closing #102 with 4b | Incremental syncs already take 1–4 s; the pain is silence, not speed | Owner Q7, Q10, Q11 |

## Scenarios
Labels:
- **S1** WHEN `about` returns a claim whose subject or object is a GitHub issue or PR with `/title` evidence at its location, THEN that side's label carries the title text, `url` = the location, the evidence id, `field` `/title`, `captured_at`, `trust_class` `untrusted content` and `clipped` false.
- **S2** WHEN `why` explains a claim, THEN its subject and object carry the same labels as in `about`.
- **S3** WHEN the entity is a Linear issue, THEN its label comes from `/title` with `url` null; WHEN it is a Linear user, THEN its label comes from `/name`, not `/displayName`.
- **S4** WHEN the entity is a repository, a comment, a GitHub account or a Person, THEN its label is null.
- **S5** WHEN a title is longer than 200 chars, with a multibyte char across the cut, THEN the label holds at most 200 whole chars and `clipped` is true.
- **S6** WHEN an entity has several title observations, THEN the label is the one with the latest `captured_at`, ties broken by the higher evidence id.
- **S7** WHEN `rastro mcp --scope <s>` hides the only title evidence of an entity, THEN its label is null and the call still succeeds; WHEN a visible copy exists in another scope, THEN that copy is used.
- **S8** WHEN labels are added, THEN claims, supports and revisions are unchanged, and `about "<a title's text>"` doesn't resolve to the entity.
- **S9** WHEN an entity has a current observed state claim (`code:state` or `tracker:state`), THEN `label.state` holds its value; WHEN it has none, THEN `label.state` is null.
- **S10** WHEN the CLI prints `about` or `why` without `--json`, THEN each labelled entity shows its label after its id with control chars escaped; WHEN `--json` is used, THEN the envelope equals the MCP result.
- **S11** WHEN an MCP client lists tools, THEN the `about` and `why` descriptions say label text is untrusted data.

Linear state:
- **S12** WHEN a Linear issue's `/state/name` evidence is extracted, THEN an observed `tracker:state` claim holds that name; WHEN the name later changes, THEN the old claim is superseded and the new one is current.

Sync progress:
- **S13** WHEN a source syncs, THEN before the read stderr names the source and its reason: `first read from <window_start>` when it has no complete check, `incremental since <window_end>` otherwise, `re-read from <window_start>` with `--reread`.
- **S14** WHEN a source syncs, THEN stderr shows the read, extract and project stages in order; WHEN extract replays stored evidence because the extractor version changed, THEN its line says so.
- **S15** WHEN a read lasts longer than 2 s, THEN stderr shows at least one running item count, and never more than one count line per 2 s.
- **S16** WHEN a source finishes, THEN stderr shows its result line with submitted, accepted and unchanged counts; WHEN it ended `rate_limited`, THEN the line gives the retry time.
- **S17** WHEN `sync --json` runs, THEN stdout holds only the one result envelope; WHEN stderr is closed or broken, THEN sync completes with the same result and exit code.

Rename warning:
- **S18** WHEN a configured source has no complete check and a stored source no longer in config has the same connector and selector, in any scope, THEN before the read stderr warns, naming both sources, and the sync result's `warnings` lists it; the read still runs from `window_start`.
- **S19** WHEN the configured source already has a complete check, or no stored-only source shares its connector and selector, THEN no warning is given and `warnings` is empty.

Switch day:
- **S20** WHEN the slice has landed, THEN the owner preps one morning's standup with `about me` over MCP without opening Linear or GitHub.

## Out of scope
- Removing or merging the duplicated `kalios` evidence (open question #9).
- A stable source id, `source rename` or `source remove`.
- Labels from scope-hidden evidence, repository names, the Linear identifier.
- #102's slow half: per-stage timings in `--json`, a profile cache across syncs (ETag), background or resumable replay.
- Hypotheses, the agent-tool adapter and model recording (slice 5).

## Risks
- **Security:** titles are untrusted content and now reach agents in one call. Covered by D1 and S1, S10, S11: the trust class travels with the label, the MCP descriptions mark label text as data, and the CLI escapes control chars. A label never feeds identity, aliases or Acts (S8).
- **Performance:** a response labels at most ~40 entities (default limit 20, subject and object). The existing `evidence_latest_by_location` index starts with `(source, scope, …)`, so a lookup by location and field across sources may need a new index on `(location, field)`. T2 measures it on the `live.rs` DB copy and adds the index only if a scoped `about me` gets slower than today.
- **Migration:** no table changes. A possible index (above) is additive. The Linear extractor version bump replays about 5.2k stored evidence rows once. That is fast (Linear read 5,233 items in 3.5 s), and S14 makes it visible.
- **Rollback:** every surface change is additive (`subject_label`, `object_label`, `warnings`, stderr lines). Reverting the PRs restores the old output. `tracker:state` claims leave on the next `claims rebuild` with the old extractor.

## Seams
- `crates/rastro-cli/tests/about_why.rs`: the binary with `--json` over seeded storage (S1–S10).
- `crates/rastro-cli/tests/mcp.rs`: MCP stdio (S7, S10, S11).
- `crates/rastro-extractor-linear` tests plus `crates/rastro-cli/tests/claims.rs` (S12).
- `crates/rastro-cli/tests/resync.rs` and `registration.rs`: the test connector, with stderr captured (S13–S19).
- `crates/rastro-cli/tests/live.rs`: owner DB copy, for the performance check.

## Tickets
The contract is `2026-11-11-answers-i-can-read.plan.yaml` (`plan/1`), with oracles in `2026-11-11-answers-i-can-read.oracle/`. In the contract, T1 is folded into T2–T5: a ticket must satisfy a scenario, so each ticket records its own decision entries. T5 also gets `schema/` for `warnings`. The table below is the original slicing.

| # | What it delivers | Satisfies | Blocked by | Paths | Size | Difficulty | Owner | Status |
|---|---|---|---|---|---|---|---|---|
| T1 | Decision entries: knowledge §69 amendment (D2, D3) and §22 note (D7); surfaces entries (D1, D6, D7's `warnings`); vocabulary `tracker:state` (D4); GLOSSARY term "Label" | none (docs) | none | docs/decisions/, docs/GLOSSARY.md | small | simple | agent | TO-DO |
| T2 | Cited labels on `about` and `why` across storage, core, operations schema, CLI text and MCP descriptions; `state` from `code:state` | S1–S11 | none | crates/rastro-storage-sqlite/, crates/rastro-core/src/knowledge.rs, crates/rastro-core/src/read_only.rs, crates/rastro-operations/, crates/rastro-cli/src/, crates/rastro-cli/tests/about_why.rs, crates/rastro-cli/tests/mcp.rs, crates/rastro-cli/tests/live.rs, schema/ | regular | complex | agent | TO-DO |
| T3 | `tracker:state` from Linear `/state/name`, with the extractor version bumped; it shows in `label.state` | S12, S9 (Linear) | T2 | crates/rastro-extractor-linear/, crates/rastro-core/src/claims.rs, crates/rastro-storage-sqlite/src/knowledge.rs, crates/rastro-cli/tests/claims.rs, crates/rastro-cli/tests/about_why.rs | small | simple | agent | TO-DO |
| T4 | Sync progress on stderr through an observer seam: start reason, stages, replay flag, throttled counts, result line with rate-limit | S13–S17 | none | crates/rastro-core/src/runs/, crates/rastro-core/src/host/, crates/rastro-operations/, crates/rastro-cli/src/main.rs, crates/rastro-cli/tests/resync.rs, crates/rastro-cli/tests/sync_search.rs, crates/rastro-test-connector/ | regular | complex | agent | TO-DO |
| T5 | Rename warning on stderr and in the sync result's `warnings` | S18, S19 | T4 | crates/rastro-core/src/runs/, crates/rastro-storage-sqlite/src/lib.rs, crates/rastro-operations/, crates/rastro-cli/src/main.rs, crates/rastro-cli/tests/registration.rs | small | complex | agent | TO-DO |
| T6 | Switch day: install from `main`, sync, prep one morning's standup with `about me` over MCP; then comment on #102 with what shipped | S20 | T1, T2, T3, T4, T5 | none | | | owner | TO-DO |
