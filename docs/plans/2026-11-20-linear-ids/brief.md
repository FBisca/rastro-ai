# Brief: Linear ids
Slug: `linear-ids`. Contract: `plan.yaml`. Lane: Plan, small · complex: a narrow change to the Linear provider and its fake server, but it changes what Rastro writes to an outside system.

## Why
Every real issue create fails. The Linear provider sends the team key (`H`) as `teamId` and `me` as `assigneeId`, and Linear wants ids for both, so it answers `INVALID_INPUT`. The fake Linear server accepted anything, so CI stayed green. It surfaced on 8b's switch day, 2026-11-20, and it blocks 8a's switch day, 8a's archive and 8b's switch day.

## What Changes
### The owner
- **New:** an issue a run creates now reaches Linear: the team key becomes its team's Linear id (S1 · T1 · `crates/rastro-action-linear/`).
- **New:** the assignee `me` becomes your Linear user id (S2 · T1).
- **New:** a team key Linear doesn't know fails the run with an error naming the key, and no issue is created (S3 · T1).
- **Unchanged:** `actions show`, `rules check` and the queue still show the team key and `me` as written.

### The system's rules
- **Unchanged:** nothing about Rastro reaches Linear. The create carries the same fields as before (title, description, team, assignee, the run's issue id), with team and assignee now as Linear ids.

### Tests
- **Changed:** the fake Linear server knows teams by key and refuses a team or assignee that isn't one of its ids (T1 · `crates/rastro-cli/tests/support/linear_api.rs`).
- **Changed:** four existing tests that asserted the raw key and `me` now assert the ids (T1).

## Capabilities
Modified spec `docs/specs/actions.md` (created by 8a's archive): adds one requirement, "A created issue names Linear's own ids".

## Impact
- Provider: `crates/rastro-action-linear/src/linear.rs` gains a query that finds a team's id by its key; `execute` in `main.rs` resolves team and `me` before the create. Undo and the issue lookup are unchanged.
- Fake server (`crates/rastro-cli/tests/support/linear_api.rs`): seeds teams such as `H` and `ENG` with ids and answers the team query.
- Fake server: refuses a `teamId` or `assigneeId` that isn't one of its ids. The viewer already answers as `user_owner`.
- Tests asserting the old raw values: `actions_accept.rs:109`, `actions_accept.rs:191`, `actions_grace.rs:179`, `actions_edit.rs:101`.
- New tests: `crates/rastro-cli/tests/actions_linear_ids.rs`.

## Context
- The store today: 2 runs, both team `H`, assignee `me`. One was stopped. The other failed all 3 attempts with `INVALID_INPUT`, so it won't be retried. No pending or accepted run is waiting on the fix.
- 8a (`docs/plans/2026-11-19-first-act/`): ACCEPTED; its switch-day ticket is still open.
- 8b (`docs/plans/2026-11-19-act-by-rule/`): ACCEPTED; its switch-day ticket is still open.
- This plan merges before either switch day.
- 8a's S12 says the team and assignee reach Linear "each unchanged". This plan reads that as "the same team and assignee" (D5).

## Example
Synthetic: fake Linear holds team `H` with id `team-h-uuid`, and the owner `user_owner`.

1. A run with team `H` and assignee `me` executes. The provider looks up `H` and gets `team-h-uuid`. The create carries `teamId: team-h-uuid` (S1).
2. The same create carries `assigneeId: user_owner`, the viewer's id (S2).
3. `actions show` on the run still reads team `H`, assignee `me`.

Variants:
- Team `ZZZ`, which fake Linear doesn't hold: no create is sent, and the run's error names `ZZZ` (S3).

## Goals / Non-Goals
Goals: a real `linear.create_issue` succeeds, for runs accepted by the owner and for runs started by a rule.

Non-goals:
- Resolving an assignee given as a name or email (D6).
- Checking the team at propose time (D7).
- Caching ids (D8).
- Rewording 8a's S12 in the spec: a one-line edit after 8a's archive (D5).

## Decisions
- D1 The provider turns the team key and `me` into Linear ids at execute time, just before the create. Rejected: core resolving them at propose time. The run's stored `team` and `assignee` are what the rule matches (`rules/mod.rs:198-199`), what the re-check compares (`runs.rs:371-372`) and what `actions show` prints, and a provider reads no evidence (ADR 0015) (gather).
- D2 The team lookup is one new fixed document, `teams(filter: {key: {eq: $key}})`. Rejected: reading team ids from the Linear connector's evidence. The provider sends only fixed documents (`linear.rs:1`) and has no evidence access (gather).
- D3 `me` resolves through the existing `Viewer` document, the same owner id undo's "another user touched it" check uses (`linear.rs` `owner()`). Rejected: a new document (gather).
- D4 The duplicate-title check is unaffected. It filters observed Linear issues locally by title and never sends the team (`actions/mod.rs:143-160`) (gather).
- D5 This plan only adds a requirement. 8a's S12 "each unchanged" reads as "the same team and assignee"; its 4 tests move to ids, and S12 is reworded in `docs/specs/actions.md` after 8a's archive creates it. Rejected: modifying S12 here (`plan check` refuses: the spec doesn't exist yet), and re-approving 8a with S12 reworded (S12's ticket is COMPLETE) (owner, 2026-11-20).
- D6 An assignee other than `me` is sent unchanged, as a Linear user id. Rejected: refusing it at propose time, and resolving names or emails. Rules only match `me` (owner, 2026-11-20).
- D7 An unknown team key fails the run at execute time, with an error naming the key. Tick's up-to-3 attempts each fail the same lookup, and nothing is created. Rejected: a propose-time check, which would need a Linear call from core (owner, 2026-11-20).
- D8 No id cache: each create sends the team lookup and `Viewer` first. Rejected: caching. Creates are a few a day, with a daily cap of 5 (owner, 2026-11-20).

## Risks / Trade-offs
- Two extra Linear calls per create. Low volume, and both are reads.
- A lookup that succeeds followed by a create that fails is retried as before. The create keeps its random issue id, so it stays exactly-once.
- The fake's refusal of non-ids is the guard that keeps this from recurring. It refuses any `teamId` or `assigneeId` it doesn't hold, so a key or `me` fails there as it does at Linear.

## Open Questions
- Is Linear's `teams(filter: {key: {eq}})` the right lookup? Not verified against the live API: the gather didn't call it. The switch day confirms it on a real create.
