# Brief: First Act
Slug: `act-by-rule`. Contract: `plan.yaml`. Lane: Plan, big · complex: rules, a new Act level, grace windows, budgets and `rules check` across storage, the catalog, tick and the levels decisions.

## Why
8a lets an agent propose a Linear issue and the owner accept it. Every issue still waits on the owner's accept, though the owner already knows which issues they'd always say yes to: their own, on their own team. 8b lets a rule the owner writes say yes for them, with a window to stop it, a cap, and undo. It is the first time Rastro acts without being asked each time, and slice 9 learns trust from these runs.

## What Changes
### The owner
- **New:** rules in `config.toml` as `[[rules]]`: name, action, team, `assignee = "me"`, optional `grace` and `daily_cap`. An invalid rule fails the command and names the rule and field (S1, S2, S3 · T1 · `crates/rastro-core/src/rules/`).
- **New:** a rule's grace is its own, else `[grace]` in config, else the table: medium, 10 min, for `linear.create_issue` (S4, S5, S6 · T1).
- **New:** `rastro tick` runs a pending Act once its grace ends, exactly once and without an accept; before that it sends nothing (S10, S11, S12 · T3 · `crates/rastro-core/src/tick.rs`).
- **New:** just before it runs, an Act is checked again: its rule still exists, no claim it cites was retracted, and no open issue has its title (S13, S14, S15 · T4).
- **New:** an Act that fails the check reads `returned` and sends nothing; its proposal waits in the queue (S13 · T4).
- **New:** `rastro actions stop <run>` ends a pending Act for good (S16 · T5 · `crates/rastro-core/src/identity.rs`).
- **New:** stopping a done run is refused and points to `undo` (S17 · T5).
- **New:** accepting, editing or rejecting a pending Act's proposal is refused and points to `stop` (S19 · T5).
- **New:** a rule's `daily_cap` counts every run it started that local day, stopped ones included. Past it, a match waits in the queue as a normal proposal (S20, S21 · T6).
- **New:** a pending Act shows first in `rastro queue`, with time left and the stop command (S22 · T7).
- **New:** `rastro rules check` shows each rule's match, grace, cap, today's count, pending Acts and the queued proposals it matches. It records nothing (S27, S28 · T8).
- **New:** `rastro actions undo` undoes a done Act run as it does an accepted one (S26 · T3).
- **New:** switch day: the owner's next real Linear issue runs at Act through their rule (S30 · T9).

### Agents
- **New:** a proposal a rule matches answers with the run, when it runs and `rastro actions stop <run>`. A proposal for another team, or one flagged as a possible duplicate, waits in the queue (S7, S8, S9 · T2 · `crates/rastro-core/src/actions/`).
- **New:** `actions show` on an Act run gives level `act`, the rule as it was when the run started, the proposal and its why, and no accept (S24, S25 · T2).
- **New:** the MCP tool `actions` lists a pending Act with when it runs and its stop command (S23 · T7 · `crates/rastro-cli/src/mcp.rs`).
- **New:** a new MCP tool `rules_check` gives the same read as the CLI (S29 · T8).
- **New:** no MCP tool stops a run or writes a rule (S18 · T5).

### The system's rules
- **Changed:** a rule the owner wrote may now authorize an Act on an agent's proposal; before, only observed or confirmed claims could (levels §17, amending §2).
- **Changed:** such a run needs no event outbox, unlike a run started from claims (knowledge §75, amending §43). Open question #13 closes.
- **New:** nothing about Rastro reaches Linear: no rule name, grace marker or why (8a's payload rule, S11).

### Rollout
- **New:** the launchd job's interval drops from 15 min to 60 s (T9). Tickets run in order T1 → T9: most share the actions, catalog and CLI files.

## Capabilities
Modified spec `docs/specs/actions.md` (created by 8a's archive): adds owner rules, grace, Act runs, stop, the daily cap and `rules check`.

## Impact
- Config: `[[rules]]` and `[grace]` on `Config` in `crates/rastro-core/src/lib.rs`, checked on load; a new module `crates/rastro-core/src/rules/`.
- Runs: a rule-started run has no accept intent, so the run table and `NewRun` gain the rule copy, level and due time; new statuses `pending`, `stopped`, `returned` (`crates/rastro-core/src/actions/runs.rs`, `crates/rastro-storage-sqlite/migrations/`).
- Intents: a new `stop` owner intent in `crates/rastro-core/src/identity.rs` and the insert-only intent table.
- Tick: runs each due Act after its re-check, before syncing stale sources (`crates/rastro-core/src/tick.rs`).
- Catalog: `rules check` read (CLI and MCP), `actions stop` CLI-only write; `schema/` regenerated; MCP gains one tool (`crates/rastro-cli/src/mcp.rs`, `crates/rastro-cli/tests/mcp.rs`, `docs/specs/surfaces.md`).
- Queue: the pending-Act line and the refusal on its proposal (`crates/rastro-core/src/queue/`).
- Tests drive time with `RASTRO_NOW` and the local day with `TZ`, against the fake Linear server in `crates/rastro-cli/tests/support/`.

## Context
- Builds on 8a (`docs/plans/2026-11-19-first-act/`): proposals, runs, exactly-once execution, retries and undo. 8a's switch day and archive must land first; the archive creates `docs/specs/actions.md`, which this plan adds to.
- The plan must be on main before T1 merges: CI's spec coverage check reads requirement titles from plans on main.
- Store today:
  - 0 action runs and 0 pending queue items;
  - 1 Linear source, scoped `work`;
  - default team `H`.

## Example
Synthetic store; rule `my-issues` (action `linear.create_issue`, team `H`, assignee `me`, `daily_cap = 5`); no `[grace]`.

1. 10:00, in a session: the agent runs `rastro actions propose linear.create_issue --title "Retry the stale sync" --team H --why "owner asked"`. It gets back run `r_3`, "runs at 10:10 unless stopped: `rastro actions stop r_3`". Linear receives nothing (S7).
2. 10:04: `rastro queue` shows first: "Retry the stale sync, runs in 6 min, `rastro actions stop r_3`" (S22). `rastro rules check` shows `my-issues` 1 of 5 used today, `r_3` pending (S27).
3. 10:10: tick checks `r_3` again, finds the rule and no same-title issue, and creates it once: `H-512` (S11).
4. Next day: `actions show r_3` gives level `act`, the rule as it was, the why, no accept (S24). `rastro actions undo r_3` deletes `H-512` (S26).

Variants:
- At 10:05 the owner runs `rastro actions stop r_3`: nothing is sent, ever (S16).
- The owner deletes the rule before 10:10: `r_3` reads `returned` and the proposal waits in the queue (S13).
- Team `ENG`, or a title matching an open issue: the proposal waits in the queue (S8, S9).
- A sixth match today: waits in the queue, "`my-issues` reached its cap of 5" (S20).

## Goals / Non-Goals
Goals: one rule-driven Act end to end for `linear.create_issue`, with grace, stop, re-check, cap, `rules check` and undo, recorded so slice 9 can learn from it.

Non-goals:
- Trust, promotion, demotion and the trust view: slice 9 (D6).
- Notifications of a pending Act: slice 11 (D7).
- Rules over observed facts or accepted commitments, and the §43 outbox (D11, D26).
- MCP writes: stop and rule edits stay CLI-only (D9, D14).
- Matching on title or description (D19).

## Decisions
- D1 No rule, no Act: a cell without an owner rule stays at Propose, over learned or default Act (levels §13).
- D2 Rules are explicit, readable config the owner writes; learning never writes rules, over learned rules (levels §8).
- D3 Grace durations come from the table: `linear.create_issue` is "others see it" × "can be undone", so medium, 10 min by default; `[grace]` in config.toml edits the table and a rule may set its own, over one fixed grace (levels §11, §15; 8a D35).
- D4 A budget is an optional per-rule daily cap, counted per local calendar day; past the cap the run falls back to Propose; unset means no cap, over a global budget (levels §14).
- D5 Undo of an Act run reuses 8a undo: delete the issue, refused when someone else touched it or its history can't be read whole, over a separate Act undo (levels §1, §16; 8a D22, D36).
- D6 Trust, promotion, demotion, the trust view and the learning journal stay in slice 9; 8b records runs, stops and undos only, over computing trust now (roadmap slice 9; learning §6–16).
- D7 Notifications and interruption budgets stay in slice 11; 8b surfaces a pending Act only where the owner already looks, over a notification now (interruptions §1, §8; roadmap slice 11).
- D8 Nothing about Rastro reaches a source: no rule name, grace marker, label, link or why goes to Linear, over tagging Act-made issues (AGENTS.md; plugins §37).
- D9 MCP stays read-only: stopping a pending Act and any rule write are CLI-only, over MCP writes (surfaces §54; 8a D8).
- D10 An action started by untrusted content is capped at Propose, and an Act whose evidence can't be verified asks instead, over acting anyway (levels §9–10).
- D11 A rule fires on an agent's proposal that it matches; the rule authorizes the Act and the agent supplies only the content. This amends levels §2. Rejected: observed facts (nothing today would match) and accepted commitments (0 in the store; the owner doesn't copy promises to Linear) (owner, round 1 Q1; open question #13).
- D12 Rules live as `[[rules]]` in `config.toml` and are checked on load: an invalid rule fails the command and names the rule. Each Act run stores a copy of the rule it ran under, over a separate rules file (owner, round 1 Q2).
- D13 During its grace window, a pending Act shows at the top of `rastro queue` and in `rastro actions` (CLI and MCP), with the time left and the stop command, over a notification (owner, round 1 Q3; D7).
- D14 `rastro actions stop <run>` is CLI-only and records a new `stop` intent; a stopped run never executes; stopping a done run is refused and points to `undo`, over stop on MCP (owner, round 1 Q4; D9).
- D15 The launchd job calls `rastro tick` every 60 s, and tick executes the Acts whose grace has ended; a tick that runs late (laptop asleep) executes after the re-check in D18, over a 15-minute cadence or a daemon (owner, round 1 Q5; plugins §35).
- D16 The budget counts every run the rule started that local day, stopped and undone ones included; past the cap, the next match waits in the queue as a normal proposal, over refunding stopped runs (owner, round 1 Q6; D4).
- D17 A run the owner accepts in the queue executes at once with no grace window; grace applies only to rule-started runs, over grace on every run (owner, round 1 Q7).
- D18 When its grace ends, an Act is checked again: the rule still exists and matches, no cited claim was retracted, and no open issue with the same title has appeared. If any check fails, it falls back to a normal proposal and nothing is sent, over acting on the facts as they were when it was proposed (owner, round 1 Q8; levels §2, §10).
- D19 A rule matches only the action, the team and an assignee of `me`; title and description are never matched. Shape: `[[rules]]` with `name`, `action`, `team`, `assignee = "me"`, optional `grace` and `daily_cap`, over matching title words (owner, round 2 Q1).
- D20 When a rule matches, `actions propose` answers that the run executes in N minutes unless stopped, with the stop command; the agent passes no flag to ask for an Act, over an explicit `--act` flag (owner, round 2 Q2).
- D21 A proposal flagged "possible duplicate" never Acts; it waits in the queue as a normal proposal, over acting with the flag shown (owner, round 2 Q3; 8a S8).
- D22 Citations stay optional for a rule-started Act; a citation the Linear source's scope can't see is still refused as in 8a, over requiring a citation (owner, round 2 Q4; 8a S6).
- D23 `rastro rules check` is a read on the CLI and MCP: for each rule, what it matches, its grace and cap, today's cap used, its pending Acts and which queued proposals it would act on now; it records nothing, and an invalid config fails it naming the rule, over a CLI-only check (owner, round 2 Q5; levels §13).
- D24 A stopped run is final: nothing returns to the queue, and the agent may propose again, over re-queuing it (owner, round 2 Q6).
- D25 Accepted risk: an agent steered by hostile content it read may propose, and Rastro can't detect it; the risk is bounded by the rule's narrow match, the grace window, the daily cap and undo, and is recorded in the levels §2 amendment, over capping every agent proposal at Propose (owner, round 2 Q7; levels §9).
- D26 Rules over agent proposals are not knowledge-derived, so they need no §43 outbox; rules over claims stay blocked until an outbox exists, noted in knowledge §43, and open question #13 closes. 8b switch day: after 8a's, an agent-proposed issue reaches Linear through the owner's rule without an accept, over leaving #13 open (owner, round 2 Q8; knowledge §43).
- D27 A pending Act's proposal can't be accepted, edited or rejected in the queue; the refusal names the run and its stop command, so one proposal never has two runs, over letting accept run it early (lead default; 8a exactly-once, knowledge §74).
- D28 A run whose re-check fails reads `returned` and its proposal waits in the queue as a normal proposal, over a silent drop (lead default; D18).
- D29 The MCP tool for the check is `rules_check`, alongside `actions`, over folding it into `actions` (lead default; surfaces §55).

## Risks / Trade-offs
- **Security, agent steering:** an agent that read hostile text may propose, and the rule would Act. Accepted (D25), bounded by the narrow match (team, assignee `me`), grace, cap and undo.
- **Duplicates:** a pending Act's proposal can't also be accepted (S19), and the 8a run id and lock keep tick to one create (S12).
- **Stale decisions:** a rule or claim may change during the grace window; the re-check (S13–S15) sends nothing when it did.
- **Missed grace:** a sleeping laptop runs the Act late, after the re-check (D15). A 60 s launchd interval costs one cheap tick a minute when nothing is due.
- **Privacy:** nothing about Rastro reaches Linear (D8); the rule copy and why stay in the run.
- **Rollback:** delete the `[[rules]]`: proposals wait in the queue as in 8a; pending runs return at their next tick (S13).

## Open Questions
- Whether the cap should refund stopped runs: decided no (D16); revisit after slice 9 data.
- Whether the owner may run a pending Act early ("accept now"): not built; stop and re-propose, or wait.
- A 60 s tick's battery and Linear rate-limit cost was not measured; the switch day checks the tick log.
