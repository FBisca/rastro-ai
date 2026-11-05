---
status: accepted
date: 2026-11-05
---

# Identity: Persons, owner intents and links

Roadmap slice 4 gives Rastro a second source and one identity per thing (`../concept.md` §1). This ADR sets what `../decisions/knowledge.md` §48 asks of it: the formulas for minting a Person and for linking to one (§36), the presentation of a group of equivalent claims (§35) and the core identity predicates (`../decisions/vocabulary.md` §14). It also sets the owner-intent table the owner writes into, how intents resolve, what each identity claim is visible in, and what a rebuild does with all of it. The plan is `../plans/2026-11-05-second-source-one-identity.plan.md`.

Terms: a **Person** is a human entity; an **account** is a source's account for a human or a bot; a **link** is an account held by a Person; a **merge** is two Persons being one (glossary, "Merge"). A name is an alias and a type is a claim (`../decisions/vocabulary.md` §15).

## Core identity predicates

The core owns the `rastro:` prefix, so the identity predicates are core (`../decisions/vocabulary.md` §14, §15):

| Predicate | Shape | Meaning |
|---|---|---|
| `rastro:held_by` | account → Person | The account belongs to the Person. A link, not a Merge |
| `rastro:same_as` | Person ↔ Person | Two Persons are one. A merge, reversible (`../decisions/knowledge.md` §34) |

- Accounts never merge with each other, and an account is never merged into a Person (`../decisions/knowledge.md` §34; glossary). A shared account may be held by two Persons (`../decisions/vocabulary.md` §14).
- `rastro:same_as` is symmetric, so its pair target is sorted; `rastro:held_by` is directed, so its pair keeps the endpoint roles.
- A cannot-link is not a claim and has no predicate. It is an owner intent of kind `cannot_link`, read by resolution and by the automatic-link veto, and listed by `rastro identity history`. It never enters the projection, so no read leaks a hidden endpoint through it. It records the owner's refusal, not a fact about the world: it applies in every scope and survives a rebuild (`../decisions/knowledge.md` §34).

## The owner-intent table

An owner intent is a durable, insert-only input, outside the rebuildable projection (`0009-claims-projection.md`). One row per accepted intent:

| Column | Contract |
|---|---|
| `id` | Portable and immutable; a retry reuses it |
| `append_seq` | Unique acceptance order |
| `format_version` | Positive; the canonical encoding version |
| `kind` | Validated by the core; no SQL enum |
| `owner_id` | The owner the intent belongs to |
| `issuer_context` | Issuing principal, surface and effective scopes. Audit data, not authorization |
| `issued_at` | When the owner issued it |
| `domain_key` | The canonical operation domain; indexed, not unique |
| `targets_json` | Versioned, tagged target structure |
| `payload_json` | Versioned operation data |
| `undo_of` | The earlier intent this one undoes; nullable self-reference |
| `supersedes` | The earlier intent this one replaces; nullable self-reference |

- No foreign key to entities, claims or occurrences: the projection is disposable, so a reference into it would break on every rebuild.
- No update and no delete. The migration that adds the table ships its insert-only triggers without a forget path: forgetting an intent waits for slice 14's controlled purge, the owner's accepted limit (plan D22). This departs from `0009-claims-projection.md`, which expected each new table with its forget path; see its 2026-11-05 amendment note.
- The kinds this slice enables are `create_person`, `name`, `link`, `cannot_link`, `same_as` and `undo`. The slice-7 kinds (claim and claim set) arrive later without a schema rewrite (`../decisions/knowledge.md` §42).

**Reserved target shapes.** The envelope is fixed now; slice 7 fills the rest.

- **Entity**: id, canonical identifier, role.
- **Pair**: two endpoints with roles; sorted only for a symmetric relation (`rastro:same_as`, and the `cannot_link` domain key).
- **Claim**: id, the reviewed revision, the full statement snapshot, a nullable versioned lifecycle binding, the anchor.
- **Claim set**: the exact unique members with those same fields, and the anchor. The group fingerprint is metadata, never a claim id (`../decisions/knowledge.md` §35).

## Resolution

Resolution is per `domain_key`, not "the latest intent wins per target". Creating, naming, linking and merging never overwrite one another; each kind has its own domain.

| Kind | Domain | Resolution inside the domain |
|---|---|---|
| `create_person` | The Person the intent mints; `me` is reserved for the owner's | One Person per domain. A second `me` returns the same Person and records no intent |
| `name` | The Person's name | Set at build (build notes) |
| `link` | The (account, Person) pair. A different Person is a different domain | One link per pair |
| `cannot_link` | The pair of endpoints, sorted | One refusal per pair |
| `same_as` | The (Person, Person) pair, sorted | One merge per pair |
| `undo` | The domain of the intent it undoes | Removes only that intent's contribution |

The exact `domain_key` formats are set at build.

- An undo names one earlier intent (`undo_of`) and removes only its contribution. The other intents stand.
- Undoing a link records a cannot-link for the (account, Person) pair; undoing a merge records one for the (Person, Person) pair. A rebuild therefore never re-creates what the owner undid (`../decisions/knowledge.md` §34).
- An explicit link or merge by the owner is the only thing that clears a cannot-link, and only for the pair it names. The refusal stays in the history, marked reversed.
- An intent is refused when it is invalid: a `held_by` from a Person, a merge of two accounts or of an entity with itself, a target that does not exist, an undo of a missing or already-undone intent, an undo of a `cannot_link` intent (only an explicit owner link or merge clears one). Nothing is stored.

## Intent-backed claims

- A Person (its `rastro:type` claim), its name, a link and a merge are claims in the projection, derived from the intent table. Their status is `observed` and their justification has kind `owner_intent`, carrying the intent's id, time and issuer; `why` shows it (`../decisions/knowledge.md` §48).
- The `confirmed` status stays in slice 7, with the scoped-replay fix it needs (`../decisions/knowledge.md` §48). Today's scoped replay forces `observed`, so this slice is consistent with it.
- **No lifecycle binding.** An identity intent targets entities, not the lifecycle state of a claim, so it carries no lifecycle binding and never lapses. This is the exemption from `../decisions/knowledge.md` §42 that §48 asks the slice-4 ADR to record. The fingerprint is defined in slice 7.

## Minting a Person

- A Person exists only through an owner intent in this slice: `rastro identity me`, `rastro identity person create`, or a yes to a candidate. The address-book and Rule paths `../decisions/knowledge.md` §36 allows wait for later.
- A Person's id is `rastro:person/<random id>`, minted by the intent that creates it. It is never derived from a name or an email (`../decisions/knowledge.md` §36).
- A Person's name is a `name` alias, written `rastro:name/<name>`. Two Persons sharing a name give the ambiguous-alias error (`../decisions/vocabulary.md` §21).
- `me` is a reserved entity form: `about me` resolves to the owner's Person. It is a fourth accepted form beside the three of `../decisions/surfaces.md` §34, and lifts `../decisions/knowledge.md` §24's "no `about(me)`" (`../decisions/surfaces.md` §38).
- `rastro identity me` mints the owner's Person once. A second run returns the same Person and records no new intent.

## Automatic links

An automatic link is not an intent: it is derived from evidence on every projection build and rebuild, never from live credentials.

**Verified email.** A verified email is a Linear workspace member's email, or one of the token owner's own verified GitHub emails (`/user/emails`, `verified: true`). A GitHub profile email is self-asserted and feeds only fuzzy candidates. Emails are stored as `email` alias claims in the source's namespace (`linear:email/…`, `github:email/…`).

**The comparison.** Two addresses are equal when the whole address matches after ASCII case-folding. No provider normalisation: no dot or plus rules (`../decisions/knowledge.md` §36).

**The formula.**

- *Email link.* Account B is held by Person P when a verified email of B equals a verified email of an account already held by P, and the two addresses are observed over an overlapping interval. The link holds while both observations hold; when the address is no longer observed on B, its interval closes and the link reads historical.
- *Token-owner link.* Each connector reports the account its token belongs to. That account is held by `me` while the token belongs to it.
- An automatic link never mints a Person. Without a link, an account is just an account.
- Equal emails over non-overlapping intervals never link.

**The veto.** A cannot-link vetoes an automatic link, and the veto holds through a `same_as` chain (transitive merges): a refusal between account B and Person P, or between B and any Person reachable from P through any number of `same_as` merges, blocks the link of B to P. Only an explicit owner link or merge clears it.

## Visibility

- A Person is visible in a scope that can see at least one of its linked accounts. `me` is visible in every scope.
- A link or a merge is visible only where both ends are visible.
- A cannot-link applies in every scope: it is a refusal, not a fact about the world, and hiding it would let an automatic link be derived again, against the owner's refusal.
- The issuer must be able to see the target when the intent is issued; an intent from a context that cannot is refused. The recorded issuing context is audit data, not lasting authorization: an intent issued in one scope applies in another that can see its target (`../decisions/knowledge.md` §42, §44).

## Rebuild, replay and cache

- The intent table sits outside the swapped projection tables (`0009-claims-projection.md`), so a rebuild never discards it and an older binary ignores it.
- A rebuild replays the evidence, carries over the pairs that cannot replay (`../decisions/knowledge.md` §50), then replays the intents, then resolves links and merges, with the cannot-link veto, before publication.
- The intent-backed claims come out identical to the incremental projection: same ids, same order, and the intent table unchanged, rows and order.
- An intent write invalidates the scoped-read cache. Today's cache is keyed by projection high-water marks (`0009-claims-projection.md`, the 2026-11-05 benchmark gate result; `../decisions/knowledge.md` §49) and has no intent watermark.

## Presenting a group

- A group of equivalent claims carries its member claim ids, each with its own status and revision; its revision is the hash of the sorted (member id, revision) pairs; it counts as one toward `limit` (`../decisions/knowledge.md` §35).
- Today only `observed` and `superseded` exist: a group with one observed member shows `observed`. Every member keeps its own status and revision in the result. How the other statuses rank in a group is set in slice 7 (`../decisions/knowledge.md` §42).
- A group is never presented as one claim with the weakest status, and there is no representative id (`../decisions/knowledge.md` §35).

## Precious inputs, export and forgetting

- Every owner intent is a precious input: insert-only, outside the projection, and exported with the other precious inputs in slice 13 (`../decisions/knowledge.md` §12, §46).
- Derived automatic links are not precious and are not exported: the observations they rest on are evidence, and a rebuild derives the links again.
- An intent keeps its copied names, identifiers and issuer context until slice 14 adds a controlled purge that leaves a marker. An accepted limit, not an oversight (`../decisions/knowledge.md` §2).

## Considered options

- **A Person per verified email:** amends §36 and mints a Person nobody asked for.
- **Nothing automatic:** an account with a verified email is left unlinked until the owner answers by hand.
- **A vocabulary-plugin Person:** Person is core (glossary; `../decisions/vocabulary.md` §2).
- **Account-to-account merge:** an account is not a human and is never merged (`../decisions/knowledge.md` §34).
- **A minimal identity-only table:** slice 7 would rewrite it. The envelope takes the slice-7 shape now, with only the identity kinds enabled.
- **Per-kind tables:** one envelope, one shape, tagged payloads; slice 7 adds kinds, not tables.
- **Foreign keys to projection rows:** the projection is disposable and rebuilt; the references would break.
- **Latest intent wins per target:** creating, naming, linking and merging would overwrite one another.
- **Binding identity intents to a fingerprint:** the fingerprint is defined in slice 7, and identity intents never lapse.
- **A `confirmed` status now:** the scoped replay forces `observed`; `confirmed` needs the slice-7 fix (`../decisions/knowledge.md` §48).
- **Intents inside the disposable projection:** an intent is a precious input and outlives the projection.
- **Visible wherever either end is:** a link in a scope that sees one end but not the other leaks the other.
- **Comparing raw strings** and **provider normalisation:** §36 forbids normalisation; the comparison is the whole address, case-folded.
- **Storing automatic links as intents:** they are derived from evidence, and a rebuild derives them again.
- **Creating `me` implicitly at the first sync:** `me` comes from an owner intent (§36).

## Deferred to slice 7

The lifecycle fingerprint, lapse and stale, the precedence rules, the `confirmed` status and the scoped-replay fix, the claim and claim-set kinds, and the expansion of an intent into several claims (`../decisions/knowledge.md` §42, §48).

## Build notes awaiting the owner's confirmation

- A Person's name: the newest `name` intent wins and the older one is superseded.
- A cannot-link between two accounts (a no to a candidate) also blocks any automatic link that would put both under one Person.
- Draft `domain_key` formats: `person/<id>` (`person/me` reserved), `person/<id>/name`, `held_by/<account>/<person>`, `cannot_link/<a>/<b>`, `same_as/<a>/<b>`.
- Every row is validated atomically: JSON shapes, non-empty ids and kinds, unique set members, a valid anchor, and a hash that agrees with the snapshot.
- The slice-13 export keeps the intents' relative acceptance order.
