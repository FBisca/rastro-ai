# Vocabulary

Status: agreed 2026-11-04. North star: `../concept.md` §1 and §4. Core terms: `../GLOSSARY.md`.

## Decisions
1. **The core holds only what its own logic needs.** Everything else is plugins.

| Layer | Holds | Who can change it |
|---|---|---|
| Core meta-model | Entity, Alias, Claim, Evidence, Justification, Coverage, Scope, Owner, Person | Core maintainers only, rarely, versioned |
| Standard vocabulary (a plugin shipped by default) | Org, Project, Commitment, Decision, Event, Conversation, Document, and their predicates (`owes`, `owed_to`, `about`, `decided_in`, `depends_on`, `supersedes`…) | That plugin's maintainers |
| Extension vocabularies | e.g. `finance:Invoice`, `health:Appointment`, `code:PullRequest` | Any plugin |

2. **Person is in the core** because reach and scopes need to know who is involved.
3. **Namespaced and additive.**
   - A vocabulary may add types and predicates, and may subtype another namespace's type (`finance:Invoice` is a `Document`).
   - It can never change or remove another namespace's meaning.
4. **Each predicate declares** its domain and range (what it links), a freshness rule, a default source authority, and a default sensitivity.
5. **The language inside vocabularies.** Every type and predicate carries:
   - a one-sentence definition;
   - "not to confuse with" (e.g. a `Commitment` is owed between people; a task is the owner's own to-do);
   - examples and counter-examples;
   - the evidence it needs.

   Extractors read these definitions, and overlap mappings are judged against them.
6. **Versioned.** A new vocabulary version means replaying the evidence, not migrating data.
7. **Overlaps** (two plugins defining the same concept) are resolved by a mapping claim that the owner or a maintainer confirms, never silently.
8. **The owner's personal language** (nicknames, "the CFO", internal IDs) is aliases and claims in their graph, not vocabulary.
9. **The core glossary** (`../GLOSSARY.md`) is the product's ubiquitous language and is protected like the core.
