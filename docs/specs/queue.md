# The commitments queue

Paths: crates/rastro-core/src/queue/, crates/rastro-core/src/read_only.rs, crates/rastro-storage-sqlite/src/, crates/rastro-operations/, crates/rastro-cli/src/, crates/rastro-cli/tests/queue.rs, schema/

One terminal queue where everything Rastro waits on the owner waits: the commitment hypotheses a conversation proposed (slice 5's `commitments` list) and the identity questions Rastro holds as same-person candidates (slice 4). Each item is one pending proposal or question with a stable id; the owner answers it in one step and Rastro records the answer as an owner intent. Renderers hold no state (`../decisions/renderers.md` §1): the queue is a view over the core, not a surface that keeps its own copy.

The queue is a read (`rastro queue`, and the MCP tool `queue`) and five owner writes on the CLI only (`accept`, `edit`, `reject`, `answer`, `missed`), because the owner's answers are owner writes and the MCP server stays read-only (`../decisions/surfaces.md` §37). The read is defined once in the operation catalog (`../adr/0007-typed-operation-catalog.md`), so the CLI and MCP cannot drift.

An item carries a **content-addressed id**, the hash of its canonical encoding, so the same proposal re-imported or replayed keeps its id and an item whose content changed is a different item. An answer names an item id; an id the queue no longer holds is refused, naming it, and nothing is recorded.

What the owner answers becomes a durable owner intent (`../adr/0010-identity-and-owner-intents.md`): accepting a commitment proposal mints one `observed` claim whose `std:` predicates carry the party names as literals (`../decisions/vocabulary.md` §32) and whose only justification is the intent. An accepted claim is visible only in scopes that can see the item's source, so a personal commitment never reaches a work-scoped agent. An action proposal is a third kind of item and mints no claim: accepting it records the intent and starts one action run in the same transaction, which creates the issue and reads `done` in `rastro actions`.

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

### Requirement: One queue of proposals and questions
The queue read SHALL return one list of every pending proposal and question the caller can see, oldest first, each item naming its source and carrying a stable id.

#### Scenario: a meeting's hypotheses are proposals
- WHEN a meeting has commitment hypotheses and the owner reads the queue
- THEN each hypothesis is one proposal item carrying a content-addressed id, its source (kind, name, scope), its kind, promise, parties, stated date, quote and standard terms, and reading the queue twice returns the same ids in the same order

#### Scenario: identity candidates are questions
- WHEN identity holds same-person candidates the caller can see
- THEN each pair is one question item carrying the same kind of content-addressed id, its two accounts with their labels and the signals that produced it, and no candidate appears twice

#### Scenario: gaps and scopes are named, never dropped
- WHEN a source found nothing, failed or could not look, or a launch scope hides a source
- THEN the hidden source's items are absent, the read still succeeds, and the result names each source's gap — `found nothing`, the failed run with its cause, or `couldn't look` with its reason

### Requirement: Everything is checked against what is known
The queue SHALL omit an item whose standard-vocabulary statement an observed claim already holds, count it, and attach the visible claims of an item's mapped parties without dropping it.

#### Scenario: an item already known is not shown
- WHEN an observed claim already holds the item's statement — the same standard predicate, party name and stated date
- THEN the item is absent from the queue and the result counts it as already known, and no name or text match ever drops an item

#### Scenario: a mapped party brings its claims
- WHEN a proposal names a party the owner mapped to an entity that has visible claims
- THEN the item carries those claims as context and stays in the queue

### Requirement: One intent per answer, recorded once
Each answer SHALL be recorded as exactly one owner intent, and an answer naming an item the queue no longer holds SHALL be refused, naming the item and recording nothing.

#### Scenario: a rejection and an answer leave the queue
- WHEN the owner rejects a proposal with a reason, or answers a question yes or no
- THEN one intent is recorded, the item leaves the queue on every surface, a read on another surface no longer shows it, and a rejection mints no claim

#### Scenario: a missed report needs no item
- WHEN the owner sends `missed` with a text, with or without an item id
- THEN one intent records the text and the item when one was named, the named item stays pending, and the report counts as missed

#### Scenario: an item that changed is refused
- WHEN an answer names an item the queue no longer holds, or names the same item a second time
- THEN the answer is refused with a reason naming the item, nothing is recorded, and the queue reads the same before and after

### Requirement: An accepted proposal becomes an observed claim
Accepting a commitment proposal SHALL record an owner intent and mint one `observed` claim carrying the item's standard terms, justified by that intent alone and citing no evidence span.

#### Scenario: accepting unchanged
- WHEN the owner accepts a proposal unchanged
- THEN one claim holds the item's statements, each standard predicate carrying the party name as a literal, `why` explains it by the owner intent, the item leaves the queue, and its outcome reads accepted

#### Scenario: accepting with edits
- WHEN the owner edits the promise or the stated date and accepts
- THEN the claim holds the owner's edited statement, the intent snapshots both the statement seen and the edited one, and the outcome reads accepted with edits rather than a clean accept

#### Scenario: an accepted claim keeps its source's scope
- WHEN a proposal from a personal-scope meeting is accepted and the queue or `about` is read with a work scope
- THEN the claim and its outcome are absent and the read succeeds, while a read without a scope shows the claim in `about me` with `why` naming the intent

### Requirement: The renderer reports what it showed
A terminal queue read SHALL record one shown report per item it printed, and no other surface SHALL record one.

#### Scenario: shown, then ignored
- WHEN the owner prints the queue and an item later leaves unanswered because a re-import replaced it
- THEN the item's outcome reads ignored with the time it waited since its first shown report, and an item still pending is never ignored however often it was shown

#### Scenario: machine reads show nothing
- WHEN the queue is read as `--json` or over MCP
- THEN no shown report is recorded, so automation cannot make an item look ignored

### Requirement: Outcomes are derived and rebuilt
The outcomes read SHALL return every outcome recorded, newest first, and a projection rebuild SHALL reproduce them and the accepted claims with no model call.

#### Scenario: reading outcomes
- WHEN the owner reads the outcomes
- THEN each carries its item id, the outcome kind, its time and the claim id when it was accepted, and the totals count what was shown and what was acted on

#### Scenario: a rebuild reproduces them
- WHEN the claims projection is rebuilt
- THEN every accepted claim returns with the same id, every outcome and shown report is reproduced, and no adapter is called

### Requirement: The queue's operations are in the catalog
The queue's read and writes SHALL be defined once in the operation catalog, the read exposed on the CLI and MCP and the writes on the CLI only, with the published schema regenerated.

#### Scenario: one catalog entry, two surfaces
- WHEN the operation schema is generated and compared with the committed one
- THEN it lists the queue read on the CLI and MCP and its five writes on the CLI only, MCP lists seven tools, and the queue read's result is a v2 envelope

#### Scenario: an agent cannot answer
- WHEN an MCP client calls a queue write
- THEN it is a protocol error and nothing is recorded

### Requirement: The queue serves a real morning
The owner SHALL work one real queue end to end from the terminal before the slice is done.

#### Scenario: switch day
- WHEN the slice has landed
- THEN the owner accepts, edits, rejects, answers and files one missed report in one sitting, and reads the outcomes afterwards without opening the meeting transcript
