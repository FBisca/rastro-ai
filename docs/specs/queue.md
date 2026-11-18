# The commitments queue

Paths: crates/rastro-core/src/queue/, crates/rastro-core/src/read_only.rs, crates/rastro-storage-sqlite/src/, crates/rastro-operations/, crates/rastro-cli/src/, crates/rastro-cli/tests/queue.rs, schema/

One terminal queue where everything Rastro waits on the owner waits: the commitment hypotheses a conversation proposed (slice 5's `commitments` list) and the identity questions Rastro holds as same-person candidates (slice 4). Each item is one pending proposal or question with a stable id; the owner answers it in one step and Rastro records the answer as an owner intent. Renderers hold no state (`../decisions/renderers.md` §1): the queue is a view over the core, not a surface that keeps its own copy.

The queue is a read (`rastro queue`, and the MCP tool `queue`) and five owner writes on the CLI only (`accept`, `edit`, `reject`, `answer`, `missed`), because the owner's answers are owner writes and the MCP server stays read-only (`../decisions/surfaces.md` §37). The read is defined once in the operation catalog (`../adr/0007-typed-operation-catalog.md`), so the CLI and MCP cannot drift.

An item carries a **content-addressed id**, the hash of its canonical encoding, so the same proposal re-imported or replayed keeps its id and an item whose content changed is a different item. An answer names an item id; an id the queue no longer holds is refused, naming it, and nothing is recorded.

What the owner answers becomes a durable owner intent (`../adr/0010-identity-and-owner-intents.md`): accepting a proposal mints one `observed` claim whose `std:` predicates carry the party names as literals (`../decisions/vocabulary.md` §32) and whose only justification is the intent. An accepted claim is visible only in scopes that can see the item's source, so a personal commitment never reaches a work-scoped agent.

Outcomes are what happened to an item — accepted, accepted with edits, rejected, answered, ignored — derived from the intents and from the shown reports the terminal read records (`../decisions/learning.md` §2–3). `ignored` is not an intent: it is an item the owner was shown and never answered, and that left the queue. The queue never shows an item whose statement an observed claim already holds (`../decisions/knowledge.md` §70 keeps hypotheses outside the projection; this reads them beside it).

## Switch day

### The RBAC Q4 queue — 2026-11-18

The owner worked the queue from the terminal, end to end, on the machine that holds his real
sources: `rastro queue` listed one identity question and the thirteen commitment hypotheses the
RBAC Q4 kickoff (`rbac-q4-2026-10-23`, scope `work`) had proposed.

He answered the question — GitHub account 48206 and Linear user `bcc46730-1cb9-4e34-8045-6073de87b1e5`,
both named Federico Feroldi — with `rastro queue answer … yes`. One `answer` intent created the
Person and held both accounts, and nothing else was linked.

He then rejected all thirteen proposals as outdated, one `reject` intent each, so no claim was
minted from them. `rastro queue` answered "Nothing is waiting"; `rastro queue outcomes` counted
fourteen items shown and fourteen acted on, each outcome naming its item, its kind and its time.

What he stops doing by hand from here on: reading meeting transcripts to find the promises he
made, and deciding same-person candidates one account pair at a time in identity. Both wait in
one queue now, and one command records the answer.

## Requirements
