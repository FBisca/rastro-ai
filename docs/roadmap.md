# Roadmap: vertical slices

A rough, living list of thin end-to-end deliverables. Every slice ends in an **outcome**: something the owner can use from that day on. Slices group into milestones; each milestone is a point where Rastro replaces something the owner does today. Order and scope change freely; `concept.md` and `decisions/` still win. Each slice gets its own plan when it starts.

Status: `todo` · `planning` · `doing` · `done`

## M1: Ask my world, with receipts
**First client use: end of slice 2.** Your agents (Claude Code, Codex) query Rastro over MCP for cited answers and honest gaps, instead of the prototype's read-only brain.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 1 | **Cited search over one real source** | Run `rastro sync` and `rastro search "x"` over a source you actually use and get evidence back with its citation (source, location, span, capture time). `rastro coverage` tells you when it couldn't look, so silence never passes for "nothing". Walking skeleton: Rust core library, CLI, append-only evidence store, the connector as an out-of-process plugin | — | knowledge §1–3, §11, §15; plugins §1, §4; surfaces §1, §8 | done (`plans/2026-11-04-cited-search-github.plan.md`, spec `specs/evidence.md`) |
| 2 | **Agents can ask** | Point Claude Code or Codex at Rastro's MCP server and have them call `search` and `coverage`, generated from the same schema as the CLI | 1 | surfaces §2–5, §11–18; plugins §6; knowledge §21 | done (`plans/2026-11-04-agents-can-ask.plan.md`, spec `specs/surfaces.md`) |
| 3 | **Claims and `about`** | Ask `about(entity)` and `why(claim)` and get observed claims from structured evidence, each traceable to its evidence | 2 | knowledge §5, §7 | done (`plans/2026-11-04-claims-and-about.plan.md`, spec `specs/claims.md`) |
| 4 | **A second source, one identity** | Add a second source and see one person seen in both as one entity. Exact identifiers link on their own, fuzzy matches ask you, and you can undo a merge | 3 | knowledge §4; vocabulary §8 | done (`plans/2026-11-05-second-source-one-identity.plan.md`, spec `specs/identity.md`) |

## M2: Commitments I can trust
**Client use: a daily commitments review** that never shows you what's already done or tracked. It replaces the prototype's proposed tasks.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 5 | **Commitments from conversations** | See commitments (asks, promises, dates, waiting-on) pulled from unstructured sources as hypotheses, each with its quoted span. Uses an agent tool adapter and a minimal standard vocabulary | 4 | knowledge §6, §13; vocabulary §1, §5; plugins §2, §10 | todo |
| 6 | **The commitments queue** | Work a terminal queue of proposals and questions, checked against what's known first: accept, edit, reject, answer, `missed`. Outcomes are recorded, and the renderer reports what it showed | 5 | renderers §1–2, §6; learning §2–3, §11 | todo |
| 7 | **Correct anything** | Confirm, fix or retract any claim; a retraction carries through to derived claims and pending proposals; "as of" queries | 6 | knowledge §7, §10; renderers §5 | todo |

## M3: It does things, safely
**Client use: Rastro acts for you** on what it has earned, with an undo, and tells you why.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 8 | **First Act** | Let one real action (e.g. create a task in your tracker) run at Propose, then at Act through a rule you write. Each Action run stores its justification and undo, with a grace window | 7 | levels (all) | todo |
| 9 | **Learned trust and usefulness** | See trust per cell grow from your outcomes: promotion is proposed to you, demotion is automatic. Includes a trust view, a learning journal, and usefulness and cost per plugin, so dead parts are visible | 8 | learning (all); plugins §9 | todo |

## M4: Where I already am
**Client use: daily rituals** in your vault and terminal, with interruptions you control.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 10 | **Notes vault renderer** | See generated views in your vault and tick to confirm or accept. Plus the opt-in "Owner notes" connector, enrolled note by note | 6 | renderers (vault) | todo |
| 11 | **Interruptions and Herdr** | Get items at the right interruption level within a budget, with batching windows, notifications, a Herdr queue pane and a "needs you" status | 6, 9 | interruptions (all); renderers (Herdr) | todo |
| 12 | **Daily rituals** | Run morning define, what changed, capture and weekly review as workflow plugins | 10, 11 | concept "Workflows" | todo |

## M5: Open and portable
**Client use: others can use it.** Friends install Rastro and its plugins, you own your data, and you can share what you choose.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 13 | **Export and rebuild** | Export evidence, confirmations, rules, outcomes and owner intents; rebuild the knowledge anywhere; swap an extractor and replay | 7 | knowledge §12, §53; sharing §1; vocabulary §6 | todo |
| 14 | **Scopes and forgetting** | Grant scopes per plugin and renderer, and forget evidence with that carrying through to everything derived from it | 7 | knowledge §2, §14; renderers §3 | todo |
| 15 | **Plugin install and permissions** | Install a plugin in one step, approve its permissions, see permission diffs on update, revoke them. Starter pack included | 2 | plugins §4–5, §11 | todo |
| 16 | **Sharing** | Share scoped claims with another owner, who receives them as evidence from a person and accepts them explicitly | 13, 14 | sharing (all) | todo |

## Parking lot
- News radar, project updates in the owner's voice, "find agent work a plain rule could do": workflows after 12.
- Windows support.
- Open questions in `open-questions.md` get decided inside the slice that first needs them.
