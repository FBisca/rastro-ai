# Glossary

The core's own language. Code, docs, plugins and the interface all use these words with these meanings. The glossary is protected the same way the core is: changes go through review.

| Term | Meaning | Not to confuse with |
|---|---|---|
| **Owner** | The person a Rastro instance serves. Owns one graph | User: anyone looking through a renderer |
| **Person** | A human entity. In the core because reach and scopes need to know who is involved | Owner |
| **Account** | A source's account for a human. The core's supertype `rastro:Account`; a vocabulary declares which of its types are accounts. Only an account is held by a Person | Person |
| **Entity** | One identified thing in the world: a person, project, commitment… | Alias |
| **Alias** | An identifier or name that points to an entity (an email, handle, nickname, ID) | |
| **Label** | The cited title or name an answer shows next to an entity, from the latest visible observation at its location, with its state. Evidence, not a claim: it never feeds an alias, a merge or a revision | Alias, Claim |
| **Merge** | A claim that two entities of the same kind are one. Reversible. An account is linked to a Person by a claim, never merged into it | |
| **Evidence** | An observation that never changes: source, location, quoted span, capture time, trust class | Source |
| **Source** | A place information comes from: a mailbox, a channel, a person, the owner's notes | Connector |
| **Trust class** | How a source's content is treated: *structured* (API records), *unstructured* (mail, chat, transcripts), *untrusted content* (can't instruct Rastro) | Trust |
| **Claim** | A statement Rastro holds about entities, with a status and at least one justification | "Fact": not a term; say claim plus its status |
| **Status** | `observed`, `hypothesis`, `confirmed`, `retracted` or `superseded` | Level |
| **Justification** | Why a claim is held: *provenance* (evidence and span), *inference* (a rule and the claims it rests on) or *confirmation* (the owner) | |
| **Confirmation** | The owner's explicit yes on a claim, given through a tick, a field or an answer | Free text the owner writes |
| **Deterministic evidence** | Observed or confirmed claims. The only basis for an Act | Hypothesis |
| **Valid time / known time** | When a claim is true in the world / when Rastro learned or unlearned it | |
| **Coverage** | A record of what was looked at, per source, scope and window: found, empty, couldn't look, never looked | |
| **Freshness rule** | How old coverage may be before the answer becomes "unknown" | |
| **Scope** | A privacy boundary on evidence and claims: work, personal, health, a friend… | |
| **Vocabulary** | A plugin's types and predicates, each with a definition, "not to confuse with", examples, and the evidence it needs | Glossary (this file) |
| **Action** | A capability a plugin declares, with its *reach* and *reversibility* | Act, Action run |
| **Reach** | Who an action affects: only me, my things, others see it, public | |
| **Reversibility** | Whether an action can be undone, compensated, or not undone at all | |
| **Level** | Act, Propose or Ask, held per action × context | Interruption level |
| **Act / Propose / Ask** | Do it visibly / prepare it and wait for a yes / ask a question | |
| **Action run** | One execution of an action, recorded with its justification and, where possible, an undo | |
| **Grace window** | An option on Act: "doing this in N minutes unless you stop me" | |
| **Context** | What a level hangs on: a person, project, source or channel. The most specific match wins | |
| **Cell** | One action × context pair, with its level and the trust behind it | |
| **Proposal / Question** | One pending Propose or Ask | |
| **Intent** | A typed input from the owner: `confirm`, `accept`, `edit`, `reject`, `answer`, `undo`, `stop`, `promote`, `missed`; identity adds `create_person`, `name`, `link`, `cannot_link`, `same_as` | |
| **Outcome** | What happened to a proposal, question, action run or notification. Recorded as evidence | |
| **Missed** | The owner reports something Rastro should have caught | |
| **Trust** | Should it act alone? Derived from outcomes, per cell | Usefulness |
| **Usefulness** | Should it do this at all? Derived from outcomes, per plugin, workflow, view or notification type | Trust |
| **Promotion / demotion** | A level going up (always proposed to the owner) or down (automatic and visible) | |
| **Pin** | An owner-frozen level that learning doesn't change | |
| **Rule** | Explicit config the owner writes. The only path to Acts that are irreversible, public, or based on shared claims | Learned level |
| **Policy pack** | A shareable plugin of rules and defaults. It can only lower levels or restrict | Rule |
| **Plugin** | An add-on of one or more kinds: connector, extractor, vocabulary, workflow, action provider, renderer, view, policy pack, agent tool adapter | |
| **Connector** | A plugin that reads a source and submits evidence and coverage | Source |
| **Extractor** | A plugin that turns evidence into claims, either deterministically or through an agent tool | Connector |
| **Workflow** | A plugin that runs a job over the knowledge, started by a trigger | Skill, subagent: agent-tool concepts, not Rastro terms |
| **Trigger** | What starts a workflow: a schedule, on demand, or a graph event | |
| **Event** | Something the core emits: new evidence, a claim changed, a retraction, a coverage gap, a proposal decided | |
| **Renderer** | A plugin that shows views and sends intents back. Holds no state | View |
| **View** | A query plus a layout plus a renderer. Defined by the owner, and shareable as a plugin | |
| **Interruption level** | How much an item may break the owner's focus: 0 silent, 1 ambient, 2 nudge, 3 notify, 4 interrupt | Level |
| **Operation** | A named read or write the core offers, defined once in the catalog and exposed on one or more surfaces. "MCP tool" is only the protocol's name for an exposed operation | Agent tool |
| **Agent tool** | Any runtime that does judgment: Claude Code, Codex, a local model… | Operation, MCP tool |
| **Agent tool adapter** | A plugin of kind `agent-tool-adapter` that lets an agent tool do judgment over content the core sends it, and answers with candidates the core validates. Its manifest declares the capabilities it needs, never a named tool | Agent tool: the runtime itself; Extractor |
| **Meeting** | One imported speaker-labelled transcript, named by an identifier the owner chooses, in one scope. Its text is untrusted content | Source: a meeting is not synced |
| **Commitment hypothesis** | A promise involving the owner that an agent tool adapter proposed from a meeting, with its exact quote. Kept outside the claims projection; never says whether the commitment is open, done or tracked | Claim with status `hypothesis` |
| **Recorded output** | The candidates a meeting extraction kept, as the extractor proposed them, recorded so a replay checks them again with no model call. Only the latest good run keeps them; never exported | Claim: it is what the extractor proposed, not a claim |
| **Supporting span** | A transcript turn a commitment hypothesis cites, kept with its text so replay can check its quote; the turns beside it keep only their speaker. Forgotten with the meeting | Evidence: kept for replay, not in the evidence store |
| **Quarantine** | A plugin that reads untrusted content can't hold action permissions | |
| **Share** | Scoped claims sent to another owner, who receives them as evidence from a person | Export |
| **Export** | Evidence, confirmations, rules, outcomes and owner intents in an open format, from which the knowledge can be rebuilt | |
