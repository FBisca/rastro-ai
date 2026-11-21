# Learned trust and usefulness (slice 9)
Lane: big × complex → Plan. New learning state across core, storage reads, CLI and MCP; sensitive (owner-trust automation, personal data).

Rastro today runs Acts only by rules you wrote; nothing learns from what you did with its proposals and runs. Slice 9 makes trust per cell grow from your outcomes: promotion is proposed to you, demotion is automatic, and a trust view, a learning journal and per-plugin usefulness/cost make dead parts visible. Cost: one repo, several agent tickets, your yes on promotions.

## Your calls
None.

## What changes

### You, at the terminal
- **New:** a trust view — every cell's level, its evidence counts, its last change and what the next promotion needs (S14, S15, S16).
- **New:** promotion proposed to you: an eligible cell shows the exact rule to add, or the reset command to restore it; nothing rises to Act without your yes (S8, S9, S10).
- **New:** demotion is automatic — an undo or a correction of a cited claim drops a level, repeated undos drop to Ask (S3, S4, S5); stopped and returned runs demote nothing (S6).
- **New:** unused trust decays a level on next use after 30 days (S7).
- **New:** an Ask cell asks a question instead of proposing; yes runs once, no rejects (S11, S12, S13).
- **New:** pin a level, reset a cell — recorded as your intents, replayed on rebuild (S18, S19).
- **New:** a learning journal — every level change with its cause and time (S17).
- **New:** per-plugin usefulness and cost — engagement, time to action, wall time plus adapter turns, cost per accepted; unseen reads "unmeasured", zero accepts reads "—" (S20, S21, S22, S23).
- **New:** thresholds configurable in `[learning]` (S1b).

### Your agents, over MCP
- **New:** trust and journal readable through the catalog gateway, scoped exactly as the CLI; the fixed tool list is unchanged (S26, S27, S28).
- **New:** every MCP read is logged; the log never marks an item seen (S24, S25).

### The system's rules
- **Changed:** a matching rule starts an Act only while the cell sits at Act; a demoted cell's match waits in the queue (S29).
- **Changed:** the pre-run recheck also checks the cell's level; a demotion during grace returns the Act (S30).
- **New:** trust is derived from outcomes and reproduced by a rebuild, with no model call (S1, S2).
- **New:** the glossary names pin, reset and undo_action as intents; the promote intent is realized as your rule edit (terminology, reviewed here).
- **New:** the switch day — a real week whose outcomes move a cell (S31).

## Walkthrough
Synthetic cell: `linear.create_issue` × team H, a rule you wrote, level Act.
1. Ten clean accepts over eight days: the trust view shows the counts and names the cell eligible (S1).
2. You undo one run: the cell drops to Propose, the journal names the undo (S3, S17).
3. The next matching proposal waits in the queue; no Act starts (S29).
4. An Act already pending when you undid returns at grace end; nothing is sent (S30).
5. A second undo in the last 20 drops the cell to Ask (S4); the next match surfaces as a question (S11); you answer yes and it runs once (S12).
6. Thirty-one days idle: the next use drops a level, the journal names the decay (S7).
7. The clean record rebuilds: the trust view proposes the reset command (S10); you reset and the baseline returns (S19). A cell with no rule is proposed the exact `[[rules]]` text instead (S8); you add it and the cell reads Act (S9).
8. The plugin's row reads engagement 6 of 10, cost 120s + 4 turns, 60s + 2 turns per accepted (S20, S22).

What can go wrong: a hidden scope's cell is absent and counts nowhere (S15); an empty store lists no cells (S16); zero accepts divides nothing (S23); a pin holds the level against demotion (S18); an MCP read never marks an item seen (S24); a rebuild answers identically (S2).

## Out of scope
- Kill rule (low usefulness → propose narrow/slow/remove): no usefulness data exists yet; slice 9 makes dead parts visible. Parked in the roadmap.
- Duplicate/stale rate per plugin: needs new per-run counters; the queue's known-first check already enforces target 0. Parked.
- Trust spreading to wider cells: cells are action × team only; no wider context exists. Parked.
- Usefulness for views and notification types: no such surface before slices 10–11. Lands there.
- Rubber-stamp guard: an experiment to measure first; stays parked in open questions.

## Risks
- **Trust acts wrongly:** demotion is automatic, but an Act still needs your rule, the grace window, the recheck and the daily cap; a pin freezes a cell. (S29, S30, S18)
- **Read logs hold personal data:** arguments are logged to your local store only, under the same scopes as the reads; nothing leaves Rastro. (S24)
- **Rebuild drift:** trust and journal derive from intents and shown reports; the rebuild scenario pins identical answers. (S2)
- **Glossary change:** the promote intent is removed as a term — deliberate, reviewed in this brief. (D15)

---

# Reference

## Build order
- T1 the trust view derives levels, counts and eligibility from outcomes (regular · complex; first).
- T2 demotion, decay and the level gate move runs automatically; the two actions requirements restated in the same PR (regular · complex; after T1).
- T3 an Ask cell's match becomes a one-step question (regular · complex; after T2).
- T4 pin/reset intents, the journal, rebuild parity; glossary amendment (regular · complex; after T2).
- T5 per-plugin usefulness and cost (regular · simple; after T1).
- T6 MCP read logging, never seen (regular · complex; independent).
- T7 trust reads on MCP through the catalog gateway (regular · complex; after T1, T4, T6).
- T8 the switch day, a real week (owner; after T3, T4, T5, T6, T7).

## Decision log

### Deferrals (owner, 2026-11-20)
**D1 Kill rule deferred.** Rejected: building the W-weeks proposal engine now. Why: no usefulness data exists yet; the slice's outcome is visibility. Source: user 2026-11-20; roadmap parking lot, `decisions/learning.md` Deferred.
**D2 Duplicate/stale rate deferred.** Rejected: per-source-run already-known counters now. Why: a run-log extension for a quality metric, not trust; known-first already enforces target 0. Source: user 2026-11-20; roadmap parking lot.
**D3 Trust spreading deferred.** Rejected: widening proposals now. Why: cells are `{action, team}`; no wider context exists to spread to. Source: user 2026-11-20; roadmap parking lot.
**D4 View/notification usefulness deferred to slices 10–11.** Rejected: measuring them in slice 9. Why: no surface exists to measure. Source: user 2026-11-20; roadmap parking lot.
**D5 Rubber-stamp guard stays parked.** Rejected: building the experiment now. Why: measure first per the decision file; it needs approval-timing data. Source: user 2026-11-20; open-questions #2.
**D6 Deferrals live in the roadmap, not only this plan.** Rejected: tracking them in the brief alone. Why: the plan archives; the parking lot keeps them visible until a slice picks them up. Source: user 2026-11-20.

### Trust model and scope (owner, 2026-11-20)
**D7 Trust is core, not a plugin.** Rejected: trust as an installable side plugin. Why: learning lives in the graph; demotion is the action engine's safety floor; the trust view is a core read like queue and focus — a renderer may render it later but cannot own it. Source: decisions/learning.md §1, renderers §9; user 2026-11-20.
**D8 Baseline and derived state (Q1 yes).** Rejected: a stored trust table; wider cells now. Why: §1/§16 make levels derived claims; team is the only context actions carry. Source: user 2026-11-20; learning §21.
**D9 Machinery cuts (Q2 yes).** Rejected: a "don't do this" intent; a tokens/money protocol change. Why: reject/undo already cover the signal; no adapter reports usage. Open question #1 closed; MCP read logging stays in as one small ticket. Source: user 2026-11-20; learning §20, §22, §24.
**D10 Demotion and decay (Q3 yes).** Rejected: stopped/returned as demotion signals. Why: nothing ran; the daily cap already counts them (levels §20). Decay default 30 days, configurable. Source: user 2026-11-20; learning §22–23.
**D11 Ask and promotion by rule (Q4 yes).** Rejected: a promotion queue item kind and a promote intent. Why: the rule is already the owner's yes (levels §8/§13); a second acceptance records the same decision twice. Source: user 2026-11-20; levels §21–22.
**D12 Usefulness and cost (Q5 yes).** Rejected: zero engagement read as 0; money costs now. Why: unseen is not ignored (§3); no usage data exists. Source: user 2026-11-20; learning §24.
**D13 Trust reads on both surfaces via the gateway (Q10 b).** Rejected: CLI-only this slice. Why: trust is core (D7), and the catalog gateway merged (#238, 8e7ece8), so there is no dependency to wait on. Source: user 2026-11-20; origin/main.
**D14 The actions spec amendments ride T2; the glossary rides T4.** Rejected: a companion plan for actions.md. Why: the engine holds one spec per plan; the gate prose and its code land in one reviewed PR, and the glossary change is reviewed in this brief. Source: planner 2026-11-20; engine plan.py.
**D15 Ask answers and re-promotion.** Rejected: a per-cell permission question; a promote intent. Why: levels §1 makes Ask a one-step answer (yes runs once, no rejects), and reset is the settled control that restores a demoted cell's baseline; the glossary's promote intent is realized as the rule edit. Source: planner 2026-11-20; levels §1, learning §14; user review.

## Code touched
Spec: `docs/specs/learning.md` (new). `docs/specs/actions.md`: two requirements restated by T2 ("A matching rule starts an Act", "An Act is checked again before it runs"). `docs/GLOSSARY.md`: intent list amended by T4.
- Derivation (new): `crates/rastro-core/src/trust/`; reads through `Snapshot` in `crates/rastro-core/src/read_only.rs`, implemented in `crates/rastro-storage-sqlite/src/`.
- Outcomes: `crates/rastro-core/src/queue/outcomes.rs` (6 kinds, derived; `queue.outcomes` slot 29 precedent).
- Runs and levels: `crates/rastro-core/src/actions/runs.rs` (`RunLevel` gains Ask), `crates/rastro-core/src/tick.rs` (recheck), `crates/rastro-core/src/actions/why.rs` (cell = {action, team}).
- Intents: `crates/rastro-core/src/identity.rs` (`IntentKind::ALL` 14 → 16: pin, reset).
- Config: `crates/rastro-core/src/lib.rs` `Config` gains `[learning]`.
- Catalog: `crates/rastro-operations/src/catalog.rs` (43 ops, slots 0–42; `deferred: true` per `actions.show` slot 37; gateway `catalog`), requests/results, `schema/operations-v2.schema.json` regenerated, drift test in `crates/rastro-operations/tests/schema.rs`.
- MCP: `crates/rastro-cli/src/mcp.rs` (gateway dispatch; read-log hook).
- Cost sources: `runs` (started_at/finished_at), `meeting_run` + `session_turn` (turns); no cost fields exist today.
- Read log: new migration in `crates/rastro-storage-sqlite/migrations/` (latest: 202611211000).
- Tests: `crates/rastro-cli/tests/` (55 files; fixtures in `support/`, fake Linear in `support/linear_api.rs`).

## Interfaces
- `rastro trust [--json]` → cells: `{action, team, level, clean, undos, last_change {cause, at}, promotion {eligible, needs, rule_text? | reset_command?}}` plus plugins: `{plugin, engagement {acted, seen} | "unmeasured", time_to_action, cost {wall_ms, turns}, cost_per_accepted | "—"}`. Visible scopes only.
- `rastro trust journal [--json]` → changes newest first: `{cell, from, to, cause, at}`.
- `rastro trust pin <action> <team> <level>` / `rastro trust reset <action> <team>` → owner intents (CLI only), replayed on rebuild.
- `[learning]` in config.toml: `clean_min = 10`, `clean_days = 7`, `undo_window = 20`, `repeat_undos = 2`, `decay_days = 30`.
- Catalog: `trust` (slot 43), `trust.journal` (44) reads, `[Cli, Mcp]`, `deferred: true`; `trust.pin` (45), `trust.reset` (46) writes, `[Cli]`. Fixed MCP list unchanged.
- Read log table: `{tool, arguments_json, at}`; insert-only; never a shown report.
- Baseline level: Act with a matching rule, Propose without; effective level = min(rule, learned trust, pin, ceilings). New contexts inherit capped at Propose (levels §6).