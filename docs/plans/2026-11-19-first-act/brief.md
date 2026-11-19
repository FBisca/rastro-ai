# Brief: First Act
Slug: `first-act`. Contract: `plan.yaml`. Lane: Plan, big/complex: a new Action subsystem (plugin role, rules, runs, grace window, undo) that writes to an external system.

## Why
Rastro reads and proposes but has never done anything outside itself. The owner creates Linear issues by asking an agent in a session, with no record of why, no undo, and nothing slice 9 could learn trust from. Slice 7 made claims correctable, so this is the first slice that can act.

## What Changes
### The owner
- **New:** an agent in any session proposes a Linear issue, and it waits in `rastro queue` with its why, team, assignee and the Linear source's scope; nothing reaches Linear yet (S4 · T2 · `crates/rastro-core/src/actions/`).
- **New:** accept it and the issue is created (S10 · T4 · `crates/rastro-action-linear/`).
- **New:** the issue is created exactly once (T8):
  - a second execution of a done run creates nothing (S11);
  - a run cut off before sending is finished by tick (S32);
  - a lost response is matched by the run's issue id, with no second create (S33);
  - an accept and a tick racing on one run send one create (S34).
- **Changed:** `rastro queue edit` on an action proposal accepts it with your changes, as it does for commitments; reject closes it with nothing run (S13, S14 · T5 · `crates/rastro-core/src/queue/answers.rs`).
- **New:** a proposal whose title matches an open Linear issue says "possible duplicate of" that issue; a Done or Canceled match is not flagged (S8, S9 · T3).
- **New:** `rastro actions undo <run>` deletes the issue (S17 · T10). Running it again after a lost response still ends `undone` with no second delete (S35 · T10). It refuses:
  - when another user commented (S18 · T11) or edited (S29 · T11);
  - when the history can't be read whole (S30 · T11);
  - when the run isn't `done` (S19 · T10).
  Your own edits and comments don't block it (S31 · T11).
- **New:** `rastro tick`, called by a macOS launchd job (T7 · `crates/rastro-core/src/tick.rs`):
  - syncs only the stale sources (S20);
  - never runs twice at once (S21);
  - keeps going when one source fails (S22).
- **New:** a failed Linear call leaves the run `failed` with its error; tick retries it until 3 attempts, then stops (S23, S24, S25 · T9).
- **New:** switch day: the owner's next real Linear issue goes through Rastro (S26 · T12).

### Agents
- **New:** `rastro actions propose <action>` needs `--why` and takes repeatable `--cite <claim id>` (T2). It is refused for:
  - an unknown action (S3);
  - a missing why (S5);
  - no team (S7);
  - a claim the Linear source's scope can't see (S6 · T3).
- **New:** the MCP tool `actions` answers "why did you do X" with the run's proposal, why, cited claims with evidence, the owner's accept, level, cell, reach and reversibility (S15 · T6 · `crates/rastro-cli/src/mcp.rs`).
- **New:** a scope other than the Linear source's sees neither the proposal nor the run (S16 · T6).

### The system's rules
- **New:** `action_provider` is a plugin kind; a provider that also submits evidence is refused (S1, S2 · T1 · `crates/rastro-core/src/host/`, ADR 0015).
- **New:** each action declares its reach and reversibility, or its provider is refused (S27 · T1).
- **New:** hard rule: the payload sent to Linear is exactly the accepted title, description, team and assignee plus the run's random id, nothing else (S12 · T4; AGENTS.md).

### Rollout
- **New:** the roadmap splits slice 8 into 8a (this plan) and 8b (rules, Act, grace window, budgets); slice 9 builds on 8b. Tickets run in order T1 → T12, because most of them share the catalog, storage and CLI files.

## Capabilities
New spec `docs/specs/actions.md`: proposing, running, explaining and undoing actions, and the `rastro tick` schedule entry point.

## Impact
- Plugin protocol: `Role` enum and `Permissions` in `crates/rastro-plugin-sdk/src/lib.rs`; the host's check that refuses any plugin holding actions, in `crates/rastro-core/src/host/mod.rs`.
- Operation catalog: new `actions.*` entries in `crates/rastro-operations/src/catalog.rs`, `requests.rs`, `results.rs` and `read.rs`. The writes are CLI-only; the read is on the CLI and MCP; `schema/` is regenerated.
- MCP goes from 7 tools to 8 (`crates/rastro-cli/src/mcp.rs`, `crates/rastro-cli/tests/mcp.rs`, `docs/specs/surfaces.md`). If focus lands too, the count is 9.
- Queue: a new item kind in `crates/rastro-core/src/queue/mod.rs`, its line in `queue_text` (`crates/rastro-cli/src/main.rs`), and accept, edit and reject for it in `queue/answers.rs`. `docs/specs/queue.md` says its claim-minting accept applies to commitment proposals.
- Intents: an undo of a run joins the insert-only intent table (`crates/rastro-core/src/identity.rs` `IntentKind`).
- Storage: new insert-only tables for proposals and runs, guarded by triggers the way `shown_report` is, registered in `crates/rastro-storage-sqlite/src/lib.rs`; config and storage interfaces in `crates/rastro-core/src/lib.rs`.
- New crate `crates/rastro-action-linear/` (workspace `Cargo.toml`), tested against a fake GraphQL server like the connector's `tests/support/api.rs`.
- Test drivers: `crates/rastro-cli/tests/support/` and the shell-script plugin pattern in `crates/rastro-cli/tests/queue.rs`.

## Context
- Gather: 2026-11-19, a fresh worker over origin/main (0dcc770, slice 7 merged) and the owner store, read-only.
- Owner store: 9,171 claims and 18 intents.
- 0 accepted commitments: all 13 meeting hypotheses were rejected as outdated on 2026-11-18.
- 356 Linear issues, 43 of them open and assigned to the owner.
- Last evidence captured 2026-11-13.
- None of these exist yet: a daemon, a scheduler, an outbox, rules, an action role, an action table.

## Example
Synthetic store; team `ENG` is `default_team`; the Linear source is scoped `work`.

1. In a Codex session the owner says "track the flaky sync test". The agent runs `rastro actions propose linear.create_issue --title "Flaky sync test" --why "owner asked in session" --cite c_91`. Claim `c_91` is visible in `work`, so it gets back item `q_7f3`, and Linear receives nothing (S4, S6).
2. `rastro queue` shows `q_7f3`: create issue "Flaky sync test" in ENG, assigned to the owner, why "owner asked in session", source Linear (`work`) (S4).
3. The owner says yes; the agent runs `rastro queue accept q_7f3`. One transaction records the accept and run `r_12` with a random issue id.
   The provider sends an issue create holding exactly the title, team, assignee and that id (S10, S12). `actions` shows `r_12 done ENG-431`.
4. The next day, an agent asks the MCP tool `actions` about `r_12`. It gets (S15):
   - the proposal and the why;
   - claim `c_91` with its evidence;
   - the accept at 10:02;
   - level `propose`, cell `linear.create_issue × ENG`, reach `others see it`, reversibility `can be undone`.
5. `rastro actions undo r_12` reads ENG-431's history, finds only the owner's changes, and deletes it; the run reads `undone` (S17, S31).

Variants:
- No `--why`, a citation only a personal scope sees, or no team: refused, nothing recorded (S5, S6, S7).
- An unknown action is refused (S3). An open issue with the same title adds a duplicate mark (S8).
- The laptop sleeps between the accept and the request: the next tick sends it once (S32). Linear created it but the reply was lost: tick finds it by id (S33).
- Linear down at accept: the run is `failed` with 1 attempt (S23), tick retries it (S24), and stops after 3 attempts (S25).
- Undo after a colleague commented or edited is refused (S18, S29), and so is undo when a history page fails (S30).

## Goals / Non-Goals
Goals: one real action end to end at Propose, with its justification, an undo, retries, and a schedule entry point. Its runs are recorded so slice 9 can learn from them later.

Non-goals:
- Act, rules, `rules check`, grace windows, `actions stop` and budgets: slice 8b (D10, D13–D15).
- The §43 outbox for runs a rule starts from claims (D21).
- MCP writes (D8).
- A harness-specific path, and moving the commitment adapter to agent-runtime (D27, D28).
- Trust, demotion and the trust view (slice 9).
- Acting on commitments (D9).

## Decisions
D1 Slice 8 next, over slice 10 or a focus slice: the owner asked to plan First Act after slice 7 (request, 2026-11-19).
D2 An Act runs only when a rule is met by observed or confirmed claims; a hypothesis or an agent's judgment alone at most proposes, over agent-authorized Acts (levels §1–2).
D3 Each Action run stores its rule, premise claims and evidence, level and its cell's trust history, and the same facts plus the same rule give the same decision, over a log of effects only (levels §3).
D4 Each action declares reach and reversibility; the level is held per cell (action × context), most specific wins; a new context inherits the general cell, capped at Propose when others see it, over a global level (levels §4–6).
D5 Promotion is proposed, never self-applied; irreversible or public Acts run only through an owner-written, readable, versioned rule; learning never writes rules, over learned promotion (levels §7–8, learning §6, §10).
D6 Untrusted content never raises a level, and a plugin that reads untrusted content holds no action permission, so the Linear write cannot live in the Linear connector, over one Linear plugin (levels §9, plugins §6, connector manifest `actions = []`).
D7 Action run outcomes (kept, stopped in grace, undone, corrected later) are derived from insert-only intents plus the run record, never an outcome table, over a stored outcome (learning §2, §16).
D8 Action writes are CLI-only; MCP stays read-only, over MCP write tools (surfaces §37, §53; focus brief D4).
D9 First action is not "an accepted commitment becomes a Linear issue", over the roadmap's example: the owner does not copy meeting promises into Linear by hand; he creates Linear issues "sometimes talking to claude" (owner, round 1 Q2, 2026-11-19), and the store holds 0 accepted commitments (gather).
D10 Slice 8 splits into 8a and 8b, each its own plan, over one plan: "split to have bigger visibility on what is being built" (owner, Q3).
D11 One idempotent entry point runs every schedule; a launchd job calls it, over per-feature timers, a daemon, or running on the next command: "a single place that runs all schedules, a proxy… it needs to be idempotent" (owner, Q4).
D12 Actions live in a new protocol role, `action_provider` (new ADR), and the Linear write in a new plugin `rastro-action-linear` that gets `LINEAR_API_KEY` through its own `permissions.env`, over writing from the connector or the core (owner, Q5; D6).
D13 A cell with no rule sits at Propose and only an owner rule raises it to Act; trust, learned demotion and the trust view wait for slice 9, over learned levels now (owner, Q7; roadmap slice 9).
D14 Budgets ship as an optional per-rule daily cap, unset meaning no cap, as a first iteration to learn from, over dropping them: the owner sees no value yet ("a guardrail unnecessary… keep as a first iteration to understand", Q8; levels §12).
D15 Grace defaults short = 2 min, medium = 10 min, long = 1 h, editable in `config.toml` `[grace]`, and a rule may set its own, over undefined durations (owner, Q9; levels §11).
D16 "Why did you do X?" is a read, `actions` and `actions show <run>`, on CLI and MCP; `rules check` is a dry run of what each rule would do now; `actions stop` and `actions undo` are CLI writes, over extending `why` (owner, Q10; levels §3; D8).
D17 8a's outcome: an in-session agent proposes a Linear issue through Rastro, the owner answers yes in the session, the agent runs the accept on the CLI, and the issue is created with its justification stored and an undo; switch day is the first issue created that way instead of directly; 8b ("First Act": rules, grace window, budgets) is its own roadmap row and slice 9 builds on it, over keeping one slice 8 (owner, round 2 Q1).
D18 What 8b may Act on (amend levels §2 for owner rules over agent requests, observed facts only, or accepted commitments) is 8b's open question, over settling it now (owner, round 2 Q2).
D19 A proposal carries a required free-text why and optional cited claim ids, stored with the run in Rastro, over an uncited request (owner, round 2 Q4).
D20 Hard invariant: nothing about Rastro is exposed to a source: an action's payload carries only the content written for that system, never Rastro ids, links, justifications, citations, footers, labels or markers; the rule goes into AGENTS.md, over a "created by Rastro" trail in the source ("THIS IS A HARD RULE INVARIANT THAT SHOULD BE ADDED ON AGENTS.md", owner, round 2 Q4).
D21 An owner-accepted proposal is not a knowledge-derived basis, so 8a needs no outbox: the accept intent and the run record commit in one transaction and execution is idempotent, so a retry never creates a second issue; recorded in knowledge.md as an amendment of §43, over building the outbox now (owner, round 2 Q5; knowledge §43).
D22 Undoing an issue create deletes it (Linear trash); when anyone else commented on or edited it since, undo is refused with the reason and the issue is left alone, over a forced delete or a state change (owner, round 2 Q6).
D23 `config.toml` `[actions.linear]` sets `default_team`; the assignee defaults to the owner; a proposal may override both (owner, round 2 Q7).
D24 An open observed Linear issue with the exact same title marks the queue item "possible duplicate of <issue>"; it warns and never blocks, over silent dropping (owner, round 2 Q8; queue known-first).
D25 The scheduler entry point (`rastro tick`) lands in 8a, syncing each source past its freshness rule; the owner installs the launchd job; 8b adds due Acts to it, over waiting for 8b (owner, round 2 Q9; D11).
D26 8a ships `actions`, `actions show <run>` and `actions undo`; `actions stop` and `rules check` go to 8b (owner, round 2 Q10; D16).
D27 Any in-session agent, from any harness, proposes through the shell (`rastro actions propose`); nothing in 8a names a harness, over a Claude-only path (owner, round 3 Q1).
D28 Every Rastro plugin that calls an agent goes through agent-runtime so the harness is a config choice; today's Claude Code commitment adapter breaks this and moves to the parking lot, not this plan, over keeping harness-specific adapters (owner, round 3 Q1: "B should also be true with any rastro plugin that calls agents").
D29 AGENTS.md gains the D20 hard rule in the owner-approved wording, and a scenario pins it on the Linear payload (owner, round 3 Q2).
D30 `rastro queue edit` on an action proposal accepts it with the owner's changes to title, description, team or assignee, the way edit already accepts a commitment proposal, over an edit that leaves a second item waiting: the owner approved editing before it runs (round 3 Q3), and the Codex review (2026-11-19) found a pending-edit meaning would contradict `queue.md` and surfaces §52–53.
D31 A failed action run is recorded `failed` with its error, shown in `actions`, and retried idempotently by `rastro tick` up to 3 attempts, then left failed, over silent or unbounded retries (owner, round 3 Q4).
D32 A run is visible only in scopes that see the source it writes to, over a run visible everywhere (owner, round 3 Q5; queue spec scope rule).
D33 A proposal is bound to the source it writes to and that source's scope; a citation that scope cannot see is refused, and the proposal, the run and its why are visible only in that scope, over filtering citations per reader: a work run citing a personal claim would leak it to a work-only reader (Codex review; knowledge §44; D32).
D34 Exactly-once execution: the run's random issue id is stored before any request, one process executes a run at a time, every execution looks the id up before creating, and tick finishes accepted runs that never completed, over retrying only `failed` runs: a crash or a lost response would otherwise strand a run or create twice (Codex review). Linear's issue create takes an optional client id (UUID v4) per its official schema (`linear/linear` `packages/sdk/src/schema.graphql`, `IssueCreateInput.id`); its reply to a repeated id is not verified, which is why lookup comes first.
D35 Each action declares reach and reversibility in its provider's manifest, and the host refuses one that doesn't; `linear.create_issue` is "others see it" and "can be undone", over names alone (levels §4; GLOSSARY "Action"; Codex review).
D36 Undo reads the issue's whole comment and change history, page by page, and refuses when any page fails, when another user commented or edited, or when the run isn't done; the owner's own changes don't block, and an undo that finds the issue already gone records `undone` without a second delete, over trusting `updatedAt` (Codex review; D22). The race between the check and the delete stays accepted.

## Risks / Trade-offs
- **Security, a new write path:** the Linear key reaches only `rastro-action-linear` through its own env grant, and a provider that also submits evidence is refused (S2), so untrusted text never reaches a plugin that can write.
- **Privacy, scopes:** a proposal lives in its source's scope; a citation that scope can't see is refused (S6), and other scopes see nothing of it (S16).
- **Privacy, sources:** the why and citations could leak into Linear. The AGENTS.md hard rule and S12 pin the payload to exactly the accepted fields.
- **Duplicates:** the run's random id is stored before any request and looked up before every create, and one process executes a run at a time (S11, S32–S34). How Linear answers a repeated id is unverified, so the lookup is what guarantees one issue.
- **Stale knowledge:** the duplicate mark reads observed issues that may be days old. `rastro tick` keeps sources fresh, and the mark only warns.
- **Undo races:** a colleague may touch the issue between the history check and the delete. The check fails closed, and the race in that window is accepted for 8a (D36).
- **Rollback:** the new tables are insert-only and additive, and removing the provider plugin disables the action.

## Open Questions
- What 8b may Act on: open question #13 (D18).
- Whether Linear's issue delete counts as an undo that others can't see, or should be a cancel. D22 chose delete; 8b's grace table may revisit it.
- Linear rate limits under tick retries were not measured; 3 attempts per run is the owner's cap, not a measured one.
- Linear's reply to a repeated client id, and the shape of its issue history API (comments, edits, pagination), are not verified live. T8 and T11 check them first against the real API, read-only where possible, and stop if they differ.
- Focus (`docs/plans/2026-11-18-focus/`) needs revising after this plan: its queue count must include action proposals without showing them, and its approval was withdrawn on 2026-11-19.
