# Roadmap: vertical slices

A rough, living list of thin end-to-end deliverables. Every slice ends in an **outcome**: something the owner can use from that day on. Slices group into milestones; each milestone is a point where Rastro replaces something the owner does today. Order and scope change freely; `concept.md` and `decisions/` still win. Each slice gets its own plan when it starts.

**Switch day.** Each slice's plan names the first day the owner uses its outcome for a real job, and what they stop doing by hand from then on. A slice is done when that day has happened, not only when its scenarios pass.

Status: `todo` · `planning` · `doing` · `done`

## M1: Ask my world, with receipts
**First client use: end of slice 2.** Your agents (Claude Code, Codex) query Rastro over MCP for cited answers and honest gaps, instead of the prototype's read-only brain.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 1 | **Cited search over one real source** | Run `rastro sync` and `rastro search "x"` over a source you actually use and get evidence back with its citation (source, location, span, capture time). `rastro coverage` tells you when it couldn't look, so silence never passes for "nothing". Walking skeleton: Rust core library, CLI, append-only evidence store, the connector as an out-of-process plugin | — | knowledge §1–3, §11, §15; plugins §1, §4; surfaces §1, §8 | done (`plans/2026-11-04-cited-search-github.plan.md`, spec `specs/evidence.md`) |
| 2 | **Agents can ask** | Point Claude Code or Codex at Rastro's MCP server and have them call `search` and `coverage`, generated from the same schema as the CLI | 1 | surfaces §2–5, §11–18; plugins §6; knowledge §21 | done (`plans/2026-11-04-agents-can-ask.plan.md`, spec `specs/surfaces.md`) |
| 3 | **Claims and `about`** | Ask `about(entity)` and `why(claim)` and get observed claims from structured evidence, each traceable to its evidence | 2 | knowledge §5, §7 | done (`plans/2026-11-04-claims-and-about.plan.md`, spec `specs/claims.md`) |
| 4 | **A second source, one identity** | Add a second source and see one person seen in both as one entity. Exact identifiers link on their own, fuzzy matches ask you, and you can undo a merge | 3 | knowledge §4; vocabulary §8 | done (`plans/2026-11-05-second-source-one-identity.plan.md`, spec `specs/identity.md`) |
| 4b | **Answers I can read** | Get `about` and `why` results whose entities carry a cited label (title or name, with its URL, and state when observed), so an agent sees what is on your plate in one call. `rastro sync` shows progress and why it is reading, and warns before a renamed source is re-read in full. Switch day: the morning after it lands, you prep standup with `about me` over MCP instead of opening Linear and GitHub | 4 | knowledge §22, §69 | done (`plans/2026-11-11-answers-i-can-read.plan.md`, spec `specs/claims.md`; switch day 2026-11-12) |

**Also shipped:** the MCP catalog gateway for deferred reads (#238); readable CLI output as labelled blocks (doing: #250, #254, #256, #259).

## M2: Commitments I can trust
**Client use: a daily commitments review** that never shows you what's already done or tracked. It replaces the prototype's proposed tasks.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 5 | **Commitments from conversations** | See commitments (asks, promises, dates, waiting-on) pulled from an unstructured source as hypotheses, each with its quoted span, through a `commitments` list on CLI and MCP. Uses an agent tool adapter and a minimal standard vocabulary; a model's interpretation is recorded and replayed, never re-called at rebuild | 4b | knowledge §6, §13, §23, §46; vocabulary §1, §5; plugins §2, §10; meetings 1–13 | done (`plans/2026-11-13-commitments.plan.yaml`, spec `specs/commitments.md`; switch day 2026-11-13) |
| 6 | **The commitments queue** | Work a terminal queue of proposals and questions, checked against what's known first: accept, edit, reject, answer, `missed`. Outcomes are recorded, and the renderer reports what it showed | 5 | renderers §1–2, §6; learning §2–3, §11 | done (`plans/2026-11-15-the-commitments-queue/plan.yaml`, spec `specs/queue.md`; switch day 2026-11-18) |
| 7 | **Correct anything** | Confirm, fix or retract any claim; a retraction carries through to derived claims and pending proposals; "as of" queries | 6 | knowledge §7, §10; renderers §5 | done (`plans/2026-11-18-correct-anything/plan.yaml`, spec `specs/corrections.md`; switch day 2026-11-18) |

**Also shipped:** the meeting task template with saved replay contracts (#237, #239, #240); meeting checks as declarative template rules (`decisions/plugins.md` §49).

## M3: It does things, safely
**Client use: Rastro acts for you** on what it has earned, with an undo, and tells you why.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 8a | **First action, proposed** | Have an agent in any session propose a Linear issue through Rastro, accept it in one step, and get it created once, with its justification kept in Rastro (never in Linear), `actions show` to ask why, and an undo. One idempotent `rastro tick`, called by launchd, keeps sources fresh and retries failed runs. Switch day: the next Linear issue you have an agent create goes through Rastro | 7 | levels §1–6, §9; knowledge §43, §74; plugins §34–36 | done (`plans/2026-11-19-first-act/plan.yaml`, spec `specs/actions.md`; switch day 2026-11-20) |
| 8b | **First Act** | Let that action run at Act through a rule you write, with a grace window you can stop, an optional daily budget, and `rules check` to see what a rule would do now | 8a | levels (all), §13–20; knowledge §75; surfaces §55 | done (`plans/2026-11-19-act-by-rule/plan.yaml`, spec `specs/actions.md`; switch day 2026-11-20) |
| 9 | **Learned trust and usefulness** | See trust per cell grow from your outcomes: promotion is proposed to you, demotion is automatic. Includes a trust view, a learning journal, and usefulness and cost per plugin, so dead parts are visible | 8b | learning (all); plugins §9 | planning (`plans/2026-11-20-learned-trust/plan.yaml`) |
| 9b | **Plugin-declared action shapes** | Add an action provider whose actions declare their own input and output as JSON Schema. The core checks a proposal before it records it and a result before it keeps it, and the CLI builds flags from the schema. The Linear fields leave the core | 8a | plugins §53; `adr/0017` | todo |

**Also shipped:** a created Linear issue carries Linear's own team and assignee ids (#204).

## M4: Every conversation
**Client use: commitments from every place you talk.** Chat threads sit next to meetings in the same commitments list and queue.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 17 | **Native Slack** | Register an approved Slack channel with a read-only token from your own Slack app, and sync its messages and thread replies as cited evidence with honest coverage. Shared, direct and hidden conversations are never read, and the token is never stored | 4b | plugins §50 | doing (`plans/2026-11-21-native-slack/plan.yaml`, spec `specs/evidence.md`) |
| 18 | **Slack identity** | See who a synced channel's people are: the token owner is your Person, each author is a Slack account with its names, and bots never link to anyone | 17 | knowledge §4; plugins §50 | planning (`plans/2026-11-21-slack-identity/plan.yaml`, spec `specs/identity.md`) |
| 19 | **Chat commitments** | See commitment hypotheses from the chat threads you opted in, next to meeting ones in the same commitments list, with speakers identified by account and threads re-read when they change. A conversation is a contract: source plugins make its turns | 5, 18 | plugins §40, §52 | planning: re-plan before T1 (`plans/2026-11-21-chat-commitments/owner-note-2026-11-21.md`) |

## M5: Connected knowledge
**Client use: follow one problem across sources.** An email, a Linear issue, a pull request and a chat thread about the same problem show as one topic, and every link cites its evidence.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 20 | **Cross-source keys** | Bind an add-on extractor beside a pre-made one, so your company's key conventions (branch names, issue keys in titles) become normalized keys without a fork | 4b | plugins §54; knowledge §77 (Tier 1); open questions #19, #20, #23 | todo |
| 21 | **Links from config** | Write join rules in config that link items whose keys match, check them with a dry run, and get each link as a cited inference claim with a predicate from the link hierarchy | 20 | knowledge §77 (Tier 2), §78 | todo |
| 22 | **Topics and chains** | Accept a topic an agent proposes, or create one, and read its members and the chain of links behind each one, walked at read time | 21 | knowledge §79, §80 | todo |

## M6: Where I already am
**Client use: daily rituals** in your vault and terminal, with interruptions you control.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 10 | **Notes vault renderer** | See generated views in your vault and tick to confirm or accept. Plus the opt-in "Owner notes" connector, enrolled note by note | 6 | renderers (vault) | todo |
| 11 | **Interruptions and Herdr** | Get items at the right interruption level within a budget, with batching windows, notifications, a Herdr queue pane and a "needs you" status | 6, 9 | interruptions (all); renderers (Herdr) | todo |
| 12 | **Daily rituals** | Run morning define, what changed, capture and weekly review as workflow plugins | 10, 11, 22 | concept "Workflows" | todo |

## M7: Open judgment
**Client use: judgment you can audit.** Agent sessions and decision adapters sort, triage and link what rules cannot. The core checks every output, and nothing becomes knowledge without your yes.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 23 | **Decision tasks** | Run task-template plugins through an agent session adapter or a decision adapter, first for duplicate/stale triage and categorization, each output validated and replayed without a model | 5 | plugins §38–43; `adr/0016` | todo |
| 24 | **Links found by judgment** | Get link hypotheses where keys and rules find nothing: deterministic narrowing to a few candidate cards, then a decision adapter, then an agent session, then you. A link becomes a claim only when you accept it | 22, 23 | knowledge §77 (Tier 3); open questions #25–#27 | todo |

## M8: Open and portable
**Client use: others can use it.** Friends install Rastro and its plugins, you own your data, and you can share what you choose.

| # | Slice | Outcome: from here on you can… | Builds on | Concept / decisions | Status |
|---|---|---|---|---|---|
| 13 | **Export and rebuild** | Export evidence, confirmations, rules, outcomes and owner intents; rebuild the knowledge anywhere; swap an extractor and replay | 7 | knowledge §12, §53; sharing §1; vocabulary §6 | todo |
| 14 | **Scopes and forgetting** | Grant scopes per plugin and renderer, and forget evidence with that carrying through to everything derived from it | 7 | knowledge §2, §14; renderers §3 | todo |
| 15 | **Plugin install and permissions** | Install a plugin in one step, approve its permissions, see permission diffs on update, revoke them. Starter pack included | 2 | plugins §4–5, §11 | todo |
| 16 | **Sharing** | Share scoped claims with another owner, who receives them as evidence from a person and accepts them explicitly | 13, 14 | sharing (all) | todo |

**Also shipped:** the published connector protocol schema and `rastro plugin check`, a part of slice 15 (#249, #251).

## Parking lot
Nothing here is decided or scheduled. A decision file wins over any line here.

- Kill rule and duplicate/stale rate: the usefulness proposal engine (narrow/slow/remove after W low weeks) and per-plugin quality counters, once slice 9's usefulness data accumulates (`decisions/learning.md`, deferred 2026-11-20).
- Trust spreading to wider cells: propose widening a cell once actions carry person, project, source or channel contexts (`decisions/learning.md` §8, deferred 2026-11-20).
- Usefulness for views and notification types: measured when their surfaces land in slices 10–11 (deferred from slice 9, 2026-11-20).
- News radar, project updates in the owner's voice, "find agent work a plain rule could do": workflows after 12.
- Windows support.
- Optional agent-runtime adapter for owners already running the fleet; default agent adapters are thin CLI drivers over a shared SDK (`decisions/plugins.md` §36, amended 2026-11-20).
- Commitments from GitHub and Linear comments: the slice-5 extractor bound to those sources, once their connectors read comments.
- Mail handed over by the owner message by message, never synced (`decisions/plugins.md` §57, 2026-11-22), once chat commitments prove useful. Meetings already shipped as slice 5's first source (`specs/meetings.md`, `adr/0011`): a local transcript import, not a work-companion integration.
- Agentic connectors (an agent chooses what to read), first for online research. Would need an amendment to plugins §13, which stands until then, and a read broker that constrains tools, scopes and arguments; evidence stays what the broker captured, and "empty" only comes from deterministic reads.
- Open questions in `open-questions.md` get decided inside the slice that first needs them.
