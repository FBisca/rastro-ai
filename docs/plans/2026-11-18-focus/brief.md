# Brief: Focus at a glance (new slice)
Slug: `focus`. Contract: `plan.yaml`. Lane: Plan, regular/complex: a new deterministic read operation + ranking across core, operations, CLI/MCP and schema, plus a personal workbench renderer in another repo.

## Why
The owner has no routine and does not visit pull surfaces (fleet logs: 1,512 sessions vs 5 /start-day runs in ~3 weeks). The use case, settled by interview 2026-11-18: **know exactly where to put his focus**, delivered concisely in a glanceable side surface, with ~30-second yes/no decisions. The assistant supplies the structure; it must not demand a habit. This slice is dependency-ready now (builds on the done queue/commitments/claims) and is the shortest path to that goal.

## What Changes

### The owner
- **New:** a FOCUS section in the Herdr workbench — one recommended focus plus up to three needs-you items, each cited (S1, S2, S10, S11 · T1, T3, T4 · personal workbench).
- **New:** start the day from that section and act without opening Linear to decide (S11 · T4 · switch day).

### The agent (Claude Code / Codex)
- **New:** a read-only MCP `focus` tool giving the same answer as the CLI (S9 · T2 · crates/rastro-operations, crates/rastro-cli).

### The system's rules
- **New:** deterministic ranking — overdue/due-today, then waiting-on-owner, then in-progress next action (S4, S5 · T1 · crates/rastro-core/src/focus.rs).
- **New:** every item cited (source, location, observed time); a typed gap when a source couldn't look; an as-of stamp (S6, S7 · T1).
- **New:** the queue shown as a count only, never proposal content (S8 · T1).
- **New:** an empty report, not an error, when nothing needs the owner (S3 · T1).
- **Changed:** the catalog and MCP tool set gain one read, `focus` (S9 · T2 · schema/operations-v2.schema.json).

### Rollout / personal
- **New:** the FOCUS card lives outside open-source rastro, in the personal workbench (S10 · T3 · ../agent-runtime/herdr/agent-workbench).
- **Prereq:** the workbench space-folder fix (agent-runtime branch `fix/workbench-space-folder`) merges before T3.

## Capabilities
`docs/specs/focus.md` — new. Six added requirements:
- A focus report ranks what needs the owner
- Focus ranking is deterministic and ordered
- Focus cites and tells the truth about gaps
- Focus shows the queue as a count
- Focus is a read in the catalog
- Focus serves a real morning

## Impact
- Core reads focus candidates from: `crates/rastro-core/src/claims.rs`, `knowledge.rs`, `read_only.rs`, `queue/`, `commitments/`; new `focus.rs` registered in `lib.rs`.
- Catalog seam: `crates/rastro-operations/src/{catalog.rs,requests.rs,results.rs,schema.rs}`; CLI `crates/rastro-cli/src/main.rs` (the `queue_text` read at line 664 is the pattern); `schema/operations-v2.schema.json`.
- Test fixtures: `crates/rastro-cli/tests/support/{mod.rs,binaries.rs}` (`rastro()`, `test_connector()`, `test_extractor()`), the `queue.rs` `Env` with its fake adapter; new `tests/focus.rs`, parity in `tests/catalog.rs`. Runs under `cargo nextest`.
- Personal renderer: `../agent-runtime/herdr/agent-workbench/{data.js,model.js,views/home.js}` reads `rastro focus --json` the way it reads `agent-orchestrator workbench --json`.

## Context
Gather: `docs/plans/2026-11-18-next-slice-gather/gather.json` (2026-11-18, a fresh agent session over rastro-ai + ../agent-fleet). Fleet: 6 agents, all in PROPOSE, 5 launchd jobs, 8 dashboard tabs; the daily ritual barely runs. Rastro today: 7 MCP tools, queue terminal-only, no mail/calendar/vault source. Prototypes: `prototype/index.html` (terminal, superseded) and `prototype/workbench.html` (the agreed card). Workbench bug found and fixed during planning: the card labelled RASTRO-AI but queried investing-portfolio because `SpaceInfo.name` follows the first terminal's live repo while `SpaceInfo.folder` is the static start folder. `spaceScope` now queries the first terminal's live cwd (128 module tests pass).

## Example
Synthetic: Linear has H-8102 (overdue by 1 day, in progress) and H-8095 (in progress); a meeting promise "doc for Ana" is due Fri; the queue holds 2 proposals; Linear's last sync failed at 07:40.
1. `rastro focus --json` reads the claims, commitments and queue (S9 · T2).
2. Ranking puts overdue H-8102 first as the recommended focus, then the waiting promise, then H-8095's next action (S1, S4 · T1).
3. Each item carries its citation and observed time. A Linear gap with its typed reason is attached, and items are stamped as-of 07:35 (S6 · T1).
4. The queue appears as count 2, no proposal text (S8 · T1).
5. The JSON holds focus, items[].citation, queue_count, gaps, as_of (S10 · T2). The workbench FOCUS card renders it (S10 · T3).
6. The owner glances, picks H-8102, acts, and records the switch day (S11 · T4).

Variants:
- no candidates → empty report, not an error (S3)
- an empty source → no gap for it (S7)
- two due-today tasks → earlier due date, then urgency, then id order (S4)
- same evidence twice → identical report (S5)
- nothing overdue but a promise waits → the promise is the focus (S2)
- CLI --json equals the MCP answer (S9)

## Goals / Non-Goals
Goals: a deterministic, cited focus read; CLI/MCP parity from one schema; a personal workbench FOCUS card; a real switch day.
Non-Goals: no mail/calendar/vault/GitHub sources (D5, D6, D8); no notifications or interruption budget (slice 11); no Act or writes — focus is read-only (D4); no model-driven ranking; no vault renderer (slice 10 deferred); no companion MCP bridge (D8, parked).

## Decisions
D1 Use case = focus direction, over fleet-replacement parity: the owner's words — "I'm not organized… I have no routine", "I know exactly where to put my focus" (owner interview, 2026-11-18).
D2 Delivery = a glanceable side surface, over session-start injection: owner Q3 "this is noise, I rather a side thing like a herdr layout… WHAT instead of WHERE" (interview). The push principle stands (zero new habits, always visible) but the mechanism is a side surface, not context injected into every session.
D3 Daily touch = ~30s yes/no inside the session, over a review ritual: owner Q5 "a" (interview).
D4 MCP stays read-only; in-session decisions run the existing CLI writes via the agent's shell, over new MCP write tools: surfaces §37/§53, queue.md (settled; no new fork).
D5 Focus v1 sources = Linear + meeting commitments + queue; GitHub out: owner "github doesn't bring much value today, no issue or discussions useful there" (interview).
D6 Vault is not a focus source: owner calls it unorganized annotations "growing exponentially… not being used for weeks", candidate for deletion (interview). Slice 10's Owner notes connector is deferred until the vault is curated; vault cleanup is a separate effort.
D7 Proposals appear in focus as a count only, over inline content: owner Q4 "a"; 13 of 14 last-switch-day proposals were outdated (queue.md switch day).
D8 Companion MCP bridge parked, over building it now: owner "heritage companion responses are text not json, it's fcked anyway" (interview). No structured read contract exists; focus v1 has no calendar/mail.
D9 Nothing is built before a visual prototype the owner agrees to, over plan-then-build: owner Q2 "we don't build anything without a visual prototype to be agreed on before" (interview).
D10 Ranking order = (1) overdue/due-today, (2) people waiting on you, (3) in-progress next action; one recommended focus + up to 3 needs-you, cited, deterministic: owner Q3 "yes" (interview).
D11 Decisions in-session/terminal is provisional, over a permanent choice: owner Q4 "for now yes until we find usefulness" (interview); revisit with usefulness data (slice 9).
D12 Delivery = a FOCUS section inside the Herdr workbench, toggled like the workbench, over a terminal sibling pane: owner Q1 "toggle like workbench, can even live inside of it… if well organized" (interview + screenshot). Coexists with fleet/orchestrate panes via the workbench's JSON contract pattern.
D13 Visual = variant B content (one focus + flat needs-you rows) in the workbench's card language, over the terminal strip: owner Q3 "ok" to B, medium corrected to workbench; agreed 2026-11-18 (prototype/workbench.html).
D14 Repo split: the generic `focus` read + ranking + --json lives in open-source rastro; the personal workbench FOCUS card lives outside rastro, over putting the Herdr glue in rastro: owner Q2 "very personal for now, lets keep out of the rastro which is a opensource" (interview).
D15 Visual prototype agreed: owner "agreed both" to the workbench card (2026-11-18), satisfying D9 (prototype/workbench.html).
D16 Workbench frontend home = agent-runtime/herdr/agent-workbench (the agent-workbench mod, per the archived herdr-app-mods README); the personal-side ticket T3 names repo ../agent-runtime.
D17 FOCUS sits below the week's KPI cards on home, over above them: owner "focus should be under the KPI Cards" (2026-11-19, after seeing the card live).
D18 Two tabs in the workbench space, Plans and Focus, over one mixed home: owner "let's do something even better, create tabs, Plans and Focus" (2026-11-19). The Focus tab is the mod with props {focus:true}: it reads only rastro focus --json (no orchestrator dependency, works in a space with no plans) and draws the report full-width; Plans is the workbench as-is.
D19 The Plans/Focus tabs live inside the workbench home, below the KPI cards, over Herdr window tabs: owner "the tabs should live below the KPI Cards" (2026-11-19). Focus is the default tab; the focus-only mod instance (D18) stays available as a whole-pane Focus surface.
D20 Focus band 2 holds only meeting promises still pending in the queue; a hypothesis whose proposal was accepted, edited or rejected leaves the report: owner "clear ... current focus" (2026-11-19) surfaced that the Nov-13 switch day's rejections kept haunting the card. A decided promise no longer waits on the owner.

## Risks / Trade-offs
- Noise in the focus line → deterministic bands, cap of three items, queue as count only (S4, S8); the ranking is the product and is tunable without a model.
- Workbench queries the wrong repo → fixed by `spaceScope` (branch fix/workbench-space-folder); T3 requires it merged.
- Personal vs open-source boundary → the generic read stays in rastro; the card stays in the personal workbench (D14); no Herdr dependency in core.
- Stale data shown as current → as-of stamp plus typed gaps (S6); empty is never a gap (S7).
- Scope creep into interruptions/Act → explicit non-goals; focus is a read.
- Rollback: focus is an additive read; removing the operation and the card restores the prior state; no existing-data migration.

## Open Questions
- "Next action" for in-progress tasks: Linear carries no next-action field, so v1 uses the task title and state; a richer next-step waits on the curated vault (deferred, D6). Assumed.
- Urgency comes from the Linear priority field via the connector; assumed present.
- Due-today is evaluated in the owner's local timezone; assumed.

unslop: 19 found, 16 applied