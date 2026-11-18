---
status: accepted
date: 2026-11-18
---

# An owner correction binds to a claim's lifecycle fingerprint, not its revision

An owner's `confirm`, `reject` or `edit` on a claim (`0010-identity-and-owner-intents.md`, the reserved `Claim` target) says something about the claim as the owner read it. It must keep holding while that reading is true, and stop holding when the claim became a different thing. The claim's **revision** (`0009-claims-projection.md`) can't carry that: it hashes every current support, so a second source corroborating the claim changes it, and a yes or a no the owner gave a minute ago would be refused or dropped for no reason the owner cares about.

## Decision
- A claim has a **lifecycle fingerprint**: the hash (versioned encoding `rastro/lifecycle/v1`) of
  - the claim id;
  - the key of its **primary live justification**, or none when no justification is live;
  - its sorted `conflicts_with` and `supersedes` links, each as (kind, other claim), including the supersession it is under.
- The **primary live justification** is the one that has held longest among the live ones (an open occurrence of evidence, a live automatic link, a live owner intent that is not itself a correction), ordered by start time and then key. A later justification never replaces it; corroboration adds a justification and leaves the primary alone. When the primary ends, the next one becomes primary and the fingerprint changes.
- The fingerprint leaves out the claim's **status**, its **revision** and every **correction**. Correcting a claim therefore never changes the state the correction binds to.
- A correction intent stores the claim id, the revision the owner saw, a snapshot of the statement, and a **lifecycle binding** (`version`, `fingerprint`). The binding is versioned so the definition can change without rereading stored intents.
- **Writing:** the claim's current revision must be the one the owner names; otherwise the command is refused as a conflict naming the claim and its current revision, and nothing is recorded. The revision is the guard against acting on something unseen; the fingerprint is what the correction then follows.
- **Following:** a retraction holds while the claim's fingerprint is the bound one, and **lapses** when it changes; a confirmation stays in force when it changes but is **stale**. Lapse, stale and what the queue asks about them land in later tickets of the same plan; this ADR fixes the state they compare.
- The renderer-facing **revision** stays the fuller hash with the occurrence history, so new corroboration changes the revision and leaves the fingerprint alone.

## Status
- A claim reads `observed`, `confirmed`, `retracted` or `superseded`. Precedence among valid statuses: retraction, then confirmation, then the evidence-derived status. `claim.status` is widened to take `confirmed` and `retracted` by a migration that rebuilds the table with foreign keys off, checked before it commits; every claim and every row naming one is kept.
- A confirmation is not a justification the owner made: it lifts the claim to `confirmed` while it stands, shows in `why` as a `confirmation` justification citing the intent's id and time, and never makes a claim visible in a scope that could not see it on other grounds. It is replayed at rebuild and in every scoped replay, so the rebuilt and scoped projections agree with the incremental one.
- In a merged Person's group, members rank confirmed, then observed, then the rest, and the group reads the strongest status among its members.

## Alternatives
- **Bind to the revision.** Simple, but any corroboration expires a correction the owner didn't expect to expire, and the rule "a correction follows its claim's lifecycle" collapses into "a correction lasts until anything changes".
- **Bind to every occurrence.** The same expiry on a finer grain: a fresh occurrence of the same evidence changes the binding.
- **Hash all live justifications.** Corroboration adds one, so the fingerprint moves; the primary alone makes corroboration invisible and an ended primary visible.
- **Edit the claim row.** The projection is rebuilt from evidence and intents; a row edit would not survive a rebuild (`../decisions/knowledge.md` §12).

## Consequences
- Correcting a claim records one insert-only intent; the projection stays rebuildable and exportable.
- The definition of the fingerprint is versioned in the intent. Changing it adds a version; stored intents keep the version they were bound to.
- Cost: the fingerprint is computed on read of one claim, from rows already indexed by claim id.
