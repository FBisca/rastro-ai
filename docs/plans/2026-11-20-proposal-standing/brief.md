# Brief: Where a run's proposal stands
Slug: `proposal-standing`. Contract: `plan.yaml`. Lane: Plan, regular · complex. It touches more than three files, and it adds a field to the `actions` result that MCP clients read.

## Why
On 2026-11-20 an Act's proposal came back into `rastro queue` after its run was done and undone (fixed in #214). The run's own read gave no sign of it, and finding the cause took a code read. If `actions show` said where the proposal stands, the mismatch would have shown on the first read.

## What Changes
### The owner
- **New:** `rastro actions show <run>` prints one `Queue:` line saying whether the run's proposal still waits in the queue (S1, S2, S3 · T1 · `crates/rastro-cli/src/main.rs`).
- **New:** a returned Act's proposal reads as waiting, and `rastro queue` lists it (S1 · T1).
- **New:** a done Act's proposal reads as out of the queue, taken out by that run being `done` (S2 · T1).
- **New:** an accepted proposal reads as out of the queue, taken out by the owner's accept, with the accept intent's id (S3 · T1).
- **Unchanged:** the run list (`rastro actions` with no run id) and what the queue holds.

### Agents (MCP)
- **New:** the MCP `actions` result for one run carries the same answer as a `queue` field of its `why` (S1, S2, S3 · T1 · `schema/operations-v2.schema.json`).
- **Unchanged:** MCP `why`, which explains claims and takes no run id (D6).

### The system's rules
- **Changed:** the queue read and the run read use one core function, so they can't disagree about where a proposal stands (D1 · T1 · `crates/rastro-core/src/queue/mod.rs`).
- **Unchanged:** nothing reaches Linear. The line and the field exist only inside Rastro.

## Capabilities
Modified spec `docs/specs/actions.md`: adds one requirement, "A run says where its proposal stands in the queue".

## Impact
- Core:
  - `crates/rastro-core/src/queue/mod.rs` gains the standing decision next to `settled_by_runs`.
  - `crates/rastro-core/src/queue/answers.rs` returns the owner intent that answered an item, not just the ids it returns today (`answered_items`).
- Run explanation: `crates/rastro-core/src/actions/why.rs` adds a field to `RunWhy`, which is filled in `explain`. That function already holds the snapshot, the run and its scope.
- CLI: `action_why_text` in `crates/rastro-cli/src/main.rs` prints the new line.
- Schema: regenerate `schema/operations-v2.schema.json` with `cargo run -p rastro-operations --bin gen-schema` (`ActionRunWhy` changes).
- Tests: a new `crates/rastro-cli/tests/actions_standing.rs`. `crates/rastro-cli/tests/actions_read.rs` too, if one of its tests compares the whole explanation.

## Context
- Today a proposal leaves the queue in two ways:
  - The owner answered it with accept, edit or reject (`answered_items`).
  - A run's status settled it (`settled_by_runs`): `accepted`, `stopped`, `done`, `failed` or `undone`.
- `pending` and `returned` runs keep the proposal in the queue. A pending Act shows first in the queue as the pending Act.
- Data on this machine:
  - 4 runs, all off the queue: 1 stopped, 1 failed, 2 undone.
  - One undone run has an accept intent, so it is both answered and settled.
  - The queue is empty.

## Example
Synthetic setup: rule `my-issues`, grace 0; fake Linear with team `H`.

1. An agent proposes "Ship the doc". The rule starts Act `r_a` on proposal `q_1`.
2. `rastro tick` after the grace creates the issue, and `r_a` reads `done`.
3. `rastro actions show r_a` prints `Queue: out, run r_a is done`. MCP `actions {"run":"r_a"}` returns `why.queue = {"state":"settled","run":"r_a","status":"done"}`. `rastro queue` doesn't list `q_1` (S2).

Variants:
- The rule was removed before tick, so `r_a` reads `returned`. The line reads `Queue: waiting for the owner`, and `rastro queue` lists `q_1` (S1).
- The owner accepts `q_2` with intent `i_9`, and its run is `r_b`. The line reads `Queue: out, the owner's accept (intent i_9)` (S3).
- `r_a` is still pending. The line reads `Queue: waiting as pending Act r_a` (D3).

## Goals / Non-Goals
Goals: the run read shows the queue's answer for its proposal, on the CLI and MCP, from the decision the queue itself uses.

Non-goals:
- A proposal status in the run list (D4).
- Run ids in MCP `why` (D6).
- Changing which statuses leave the queue: #214 settled that.

## Decisions
- D1 One core function returns where a proposal stands, from all of the proposal's runs plus the owner's intents. The queue's filter and the run read both call it, and its match covers every run status. Rejected: the run read keeping its own copy of the rule, since a drifting copy is exactly how the #214 bug stayed hidden (gather: `queue/mod.rs:1050`, `answers.rs:80`).
- D2 There are four states:
  - `answered` (the intent's kind and id)
  - `settled` (the run that settled it and its status)
  - `pending_act` (the pending run)
  - `waiting`

  Precedence follows that order, so an accepted run that is also done reads `answered`. Rejected: run status first. The owner's answer is the more direct reason, and for a propose-level run it always exists (owner-delegated, 2026-11-20; gather edge case: one real undone run has an accept intent).
- D3 A pending Act reads `pending_act`, not `waiting`. The queue shows it as the pending Act with a stop command, not as a plain proposal. Rejected: folding it into `waiting` (owner-delegated, 2026-11-20).
- D4 The field lives on `RunWhy` (one run's read) as `queue`, not on `Run`. Rejected: adding it to every row of the list, which the request didn't ask for and which would make each list row read intents (owner-delegated, 2026-11-20; gather: `actions/mod.rs:402`).
- D5 The CLI prints one line after `Accepted:`/`Rule:` and before `Level:`, in this exact wording:
  - `Queue: waiting for the owner`
  - `Queue: waiting as pending Act <run>`
  - `Queue: out, the owner's <accept|edit|reject> (intent <id>)`
  - `Queue: out, run <run> is <status>`

  Rejected: a new glossary term for the states. "Queue" is already a term (owner-delegated, 2026-11-20; `docs/GLOSSARY.md:62`).
- D6 MCP `why` stays claim-only. The run's queue state rides in the `actions` result, which is the MCP read of one run. Rejected: making `why` accept a run id (gather: `requests.rs:185`).
- D7 When several runs settled the same proposal, `settled` names the latest settling run. Rejected: listing all of them (owner-delegated, 2026-11-20).

## Risks / Trade-offs
- An extra read of the intents and runs per `actions show`. This is a single-run read, so the cost is small.
- Clients that compare the whole `actions` JSON see a new `why.queue` key. The change only adds a field, and the schema is regenerated in the same PR.
- Scope: the decision reads runs in the caller's scopes, as the queue does. A run in another scope stays invisible.

## Open Questions
- A returned run's proposal that the owner later accepts gets a second run. The spec doesn't say whether that is intended. This plan reports it as `answered` and doesn't decide it.
