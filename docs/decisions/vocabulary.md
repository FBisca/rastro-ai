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

## Added 2026-11-04 (roadmap slice 3)
Architecture: `../adr/0008-extractor-role-and-protocol.md`.
10. **The `code` vocabulary ships in the GitHub extractor package,** as a file its manifest names and its `dependencies` list. Each predicate carries §5's definition fields, an evidence requirement, domain and range, freshness (the source's), authority (the newer comparable observation, `knowledge.md` §26) and sensitivity.
11. **Predicates this slice:** `code:state` (`open` or `closed` exactly as the source defines it, so a merged pull request is `closed`), `code:author`, `code:in_repository`. Labels, assignees, merged, comment parent, review requested and mentions wait.
12. **Types:** `code:Account` for a GitHub user, `code:Bot` for a bot, `code:Repository`, `code:PullRequest`, `code:Issue`, `code:Comment`. Nothing is typed `Person` (§2): an account type doesn't prove a human. Slice 4 links accounts to Persons.
13. **Cardinality is per subject and observation context,** not global. Contradictions are kept as conflicts (`knowledge.md` §26), never rejected as cardinality violations.

## Added 2026-11-05 (knowledge graph deep dive)
Architecture: amendments of 2026-11-05 to `../adr/0008-extractor-role-and-protocol.md`.
14. **Accounts are not humans (clarifies §2 and §12).** A Person hub: an account is linked to a Person by a claim; there is no literal account-equals-account rule, and a shared account may link to two Persons. The core meta-model gains reserved predicates for this, under the core's own `rastro:` prefix (such as `rastro:held_by` and `rastro:same_as`); their exact set is named in the slice 4 ADR. A link is a claim, not a Merge (glossary).
15. **A type is a claim, not a column (amends §1).** An entity's type is a `rastro:type` claim; its aliases are `rastro:alias` claims with intervals. Nothing is updated in place. Subtyping (an optional `supertype` in a vocabulary) decides whether two types conflict. An old login stays a historical lookup candidate; a reused login is ambiguous and never merges. The core owns the `rastro:` prefix; a plugin namespace can't use it. Which plugin owns a prefix when two vocabularies collide is decided at slice 16 (`sharing.md` §8).
16. **Cardinality is report-only (clarifies §13).** Two live values for a scalar make a conflict, never a rejection. Every predicate declares `cardinality`; there is no default.
17. **Validation is fixed checks, not a rules language (clarifies §4 and §5).** A vocabulary file with an unknown field is rejected. The core validates a submission as one bundle: endpoint types come from type claims in the same bundle, not from stored state. A conflicting type from another bundle is a visible conflict, subject to subtyping. Overlap mappings remain confirmed claims (§7). A definition change means replay (§6). Every new vocabulary feature is core code; no SHACL-style engine for now.
18. **Datatypes keep lexical identity** (`"01"` is not `"1"`; `date` is not `datetime`). Canonical forms per datatype may be added later under an explicit id version.
19. **No standards profile yet.** Claims aren't exported (`sharing.md` §1 exports precious inputs). RDF, PROV, nanopublication or JSON-LD profiles and IRIs wait for a consumer; see `sharing.md` §8.

## Added 2026-11-05 (claims hardening)
Architecture: amendments of 2026-11-05 to `../adr/0008-extractor-role-and-protocol.md` and `../adr/0009-claims-projection.md`.
20. **Alias lookup counts every status except `hypothesis` and `retracted` (clarifies §15).** Today that is `observed` and `superseded`; `confirmed` joins from slice 7. A superseded alias is the historical candidate of §15, so an old login still resolves.
21. **A login held by several entities is the ambiguous-alias operation error (clarifies §15).** It lists every candidate, marked `current` (observed or confirmed) or `historical` (superseded). A single holder resolves directly. For a scoped read the marks come from visible justifications in the replay, never from stored status, which is full-scope (`knowledge.md` §44). Nothing merges.
22. **Type conflict is entity-wide, for `rastro:type` only (clarifies §16).** Two live type claims of one entity conflict wherever they sit, unless one type is an ancestor of the other (§15). Aliases and every other predicate stay per (location, field): an old login not yet re-read must not conflict with the new one, and other scalars have the item location as subject. Dominance, supersession and slot selection stay per (location, field), because item-level times are not comparable across items (`knowledge.md` §26). Mechanism: `../adr/0009-claims-projection.md`.
23. **Alias kind and value are checked (extends §17).** Vocabulary load refuses an empty alias kind or one containing `/`, `:` or whitespace; submit rejects an alias value that is empty, whitespace-only, or has whitespace at either end; inner whitespace is allowed (`Felippe Bisca`), as in the entity forms (`surfaces.md` §34), because `rastro:alias` is core, not GitHub: names from later sources contain spaces. Otherwise a stored alias could not be written as a canonical alias (`surfaces.md` §31) and `about` could not reach it.
    - *Amended 2026-11-05:* the owner decided inner whitespace in the value on 2026-11-05; the first wording was no whitespace anywhere.

## Added 2026-11-05 (roadmap slice 4)
Architecture: `../adr/0010-identity-and-owner-intents.md`. Plan: `../plans/2026-11-05-second-source-one-identity.plan.md`.
24. **The core's identity predicates are named (decides what §14 left open).** Under the core's own `rastro:` prefix: `rastro:held_by` (an account is held by a Person) and `rastro:same_as` (two Persons are one, reversible). A cannot-link is an owner intent, not a predicate (`../adr/0010-identity-and-owner-intents.md`). Accounts never merge with each other, and an account is linked to a Person by a claim, never merged into it (§14; glossary "Merge"). `rastro:Person` is a core type, claimed like any other (§15), minted only by an owner intent in this slice (`knowledge.md` §54).
25. **The alias kinds `name` and `email`.** A `name` is a Person's name, written `rastro:name/<name>` in the core namespace; a name two Persons share gives the ambiguous-alias error (§21). An `email` is an address observed on an account, written in the source's namespace (`linear:email/…`, `github:email/…`). A verified email justifies a link; an unverified one is only a candidate (`../adr/0010-identity-and-owner-intents.md`).
26. **The `tracker` vocabulary,** shipped in the Linear extractor package, like `code` in the GitHub one (§10). Types `tracker:Account` (a workspace user) and `tracker:Issue`; identifiers `linear:user/<uuid>` and `linear:issue/<uuid>`. A workspace user is an account, never a Person (§12).
