---
status: accepted
date: 2026-11-05
---

# Identity: Persons, owner intents and links

Roadmap slice 4 gives Rastro a second source and one identity per thing (`../concept.md` §1). This ADR sets what `../decisions/knowledge.md` §48 asks of it: the formulas for minting a Person and for linking to one (§36), the presentation of a group of equivalent claims (§35) and the core identity predicates (`../decisions/vocabulary.md` §14). It also sets the owner-intent table the owner writes into, how intents resolve, what each identity claim is visible in, and what a rebuild does with all of it. The plan is `../plans/2026-11-05-second-source-one-identity.plan.md`.

Terms: a **Person** is a human entity; an **account** is a source's account for a human; a **link** is an account held by a Person; a **merge** is two Persons being one (glossary, "Merge"). A name is an alias and a type is a claim (`../decisions/vocabulary.md` §15).

## Core identity predicates

The core owns the `rastro:` prefix, so the identity predicates are core (`../decisions/vocabulary.md` §14, §15):

| Predicate | Shape | Meaning |
|---|---|---|
| `rastro:held_by` | account → Person | The account belongs to the Person. A link, not a Merge |
| `rastro:same_as` | Person ↔ Person | Two Persons are one. A merge, reversible (`../decisions/knowledge.md` §34) |
| `rastro:verified_email` | account → email literal | An address the source verifies for this account |
| `rastro:token_owner` | account → `true` | The source token belongs to this account |

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
| `issuer_context` | Issuing principal, surface and configured source scopes. Audit data, not authorization; see D35 below |
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
| `name` | The Person's name | Newest non-undone name wins; earlier stretches stay historical (D39 below) |
| `link` | The (account, Person) pair. A different Person is a different domain | One link per pair |
| `cannot_link` | The pair of endpoints, sorted | One refusal per pair |
| `same_as` | The (Person, Person) pair, sorted | One merge per pair |
| `undo` | The domain of the intent it undoes | Removes only that intent's contribution |

The exact `domain_key` formats are set at build.

- An undo names one earlier intent (`undo_of`) and removes only its contribution. The other intents stand.
- Undoing a link records a cannot-link for the (account, Person) pair; undoing a merge records one for the (Person, Person) pair. A rebuild therefore never re-creates what the owner undid (`../decisions/knowledge.md` §34).
- An explicit link or merge by the owner is the only thing that clears a cannot-link, and only for the pair it names.
- An intent is refused when it is invalid: a `held_by` from a Person, a merge of two accounts or of an entity with itself, a target that does not exist, an undo of a missing or already-undone intent, an undo of a `cannot_link` intent (only an explicit owner link or merge clears one). Nothing is stored.

## Candidates and answers (2026-11-06)

Candidates are computed on read over every scope and never stored, either as claims or as intents (`../decisions/knowledge.md` §55, §63–64; CLI contract in `../decisions/surfaces.md` §37). Only answers are stored, using the existing identity intent kinds: a no is `cannot_link` between the two accounts; a yes is `link` for each unheld account to the existing holder, or `create_person`, `name` and two `link` intents when neither is held. Merged Persons count as one holder. The pair must be a candidate now; a yes with different, unmerged holders is refused. All intents for an answer are accepted together, or none is.

There is no separate candidate id, answer intent kind or answered marker in the payload. Undo uses the ordinary link semantics above, so a formerly answered pair can become a candidate again when it is no longer settled. Naming and refusal rules live in `../decisions/knowledge.md` §65–67.

## Intent-backed claims

- A Person (its `rastro:type` claim), its name, a link and a merge are claims in the projection, derived from the intent table. Their status is `observed` while current and `superseded` when undone or replaced; their justification has kind `owner_intent`, carrying the intent's id, time and issuer; `why` shows it (`../decisions/knowledge.md` §48).
- The `confirmed` status stays in slice 7, with the scoped-replay fix it needs (`../decisions/knowledge.md` §48). Identity replay preserves `observed` and `superseded`; the general status fix remains deferred.
- **No lifecycle binding.** An identity intent targets entities, not the lifecycle state of a claim, so it carries no lifecycle binding and never lapses. This is the exemption from `../decisions/knowledge.md` §42 that §48 asks the slice-4 ADR to record. The fingerprint is defined in slice 7.

## Minting a Person

- A Person exists only through an owner intent in this slice: `rastro identity me`, `rastro identity person create`, or a yes to a candidate. The address-book and Rule paths `../decisions/knowledge.md` §36 allows wait for later.
- A Person's id is `rastro:person/<random id>`, minted by the intent that creates it. It is never derived from a name or an email (`../decisions/knowledge.md` §36).
- A Person's name is a `name` alias, written `rastro:name/<name>`. Two Persons sharing a name give the ambiguous-alias error (`../decisions/vocabulary.md` §21).
- `me` is a reserved entity form: `about me` resolves to the owner's Person. It is a fourth accepted form beside the three of `../decisions/surfaces.md` §34, and lifts `../decisions/knowledge.md` §24's "no `about(me)`" (`../decisions/surfaces.md` §38).
- `rastro identity me` mints the owner's Person once. A second run returns the same Person and records no new intent.

## Automatic links

An automatic link is not an intent: it is derived on every projection build and rebuild, never from live credentials. Extractors emit `rastro:verified_email` and `rastro:token_owner` claims from evidence. The core derives links only from these claims, `me` and owner intents; it does not interpret plugin evidence kinds or name plugin types. Premise intervals and scope visibility come from the claim lifecycle (D44, accepted 2026-11-05).

**Verified email.** A verified email is a Linear workspace member's email, or one of the token owner's own verified GitHub emails (`/user/emails`, `verified: true`). Guests, bots and GitHub profile emails never supply `rastro:verified_email`; Rastro does not read GitHub profile emails at all (narrowed 2026-11-06, owner Q46): they are self-asserted and often empty, and as aliases they made the token owner's verified-email alias ambiguous. Candidates rely on names and logins. Emails are also stored as `email` alias claims, displayed as `<namespace>:email/<address>`.

**The comparison.** Two addresses are equal when the whole address matches after ASCII case-folding. No provider normalisation: no dot or plus rules (`../decisions/knowledge.md` §36).

**The formula.**

- *Email link.* Account B is held by Person P when a verified email of B equals a verified email of an account already held by P, and the two addresses are observed over an overlapping interval. The link holds while both observations hold; when the address is no longer observed on B, its interval closes and the link reads historical.
- *Token-owner link.* An account with a live `rastro:token_owner` claim is held by `me` over the claim's interval, once `me` exists.
- An automatic link never mints a Person. Without a link, an account is just an account.
- Equal emails over non-overlapping intervals never link.

**Snapshot evidence (D46, accepted 2026-11-05; D49, accepted 2026-11-06).** The token owner's verified addresses are one evidence record at `/emails/verified`: a JSON-list header (including `[]` when empty), followed by raw addresses with a separate span per address. Each email alias and `rastro:verified_email` claim cites its address's span in that record, quoting the exact unescaped address even when JSON escapes it. Spans never outlive their snapshot. A failed read writes no snapshot and keeps the previous one.

**Snapshot replacement (D47, accepted 2026-11-06).** A newer snapshot from the same source and selection replaces the older one, closing claims supported only by the older snapshot. This exception applies only to `rastro:verified_email`, `rastro:token_owner` and email-alias claims; every other claim keeps its lifecycle. A dropped address or an empty list therefore closes its alias and any link whose premises no longer hold.

**Inference storage.** Automatic links are inference justifications in the rebuildable `automatic_justification` table: id, claim id, reason (`verified_email` or `token_owner`), premise claim ids (`premise_a`, nullable `premise_b`), and `started_at`/`ended_at`. A token-owner link cites its token-owner claim; an email link cites both verified-email claims.

**The veto.** A cannot-link vetoes an automatic link, and the veto holds through a `same_as` chain (transitive merges): a refusal between account B and Person P, or between B and any Person reachable from P through any number of `same_as` merges, blocks the link of B to P. Only an explicit owner link or merge clears it.

**Order independence (D48 and D51, accepted 2026-11-06).** An owner link always wins and vetoes an automatic link that would join a cannot-linked account. If two automatic links qualify for one Person but their accounts are cannot-linked, neither joins during their qualifying overlap; outside that overlap each may join. The veto checks all accounts of Persons joined through `same_as`, rather than choosing by ingest or projection order.

Conflicting automatic links are blocked only over the time both qualify, and a scoped read keeps the same windows (`../specs/identity.md`).

## Visibility

- A Person is visible in a scope that can see at least one of its linked accounts. `me` is visible in every scope.
- A link or a merge is visible only where both ends are visible.
- A cannot-link applies in every scope: it is a refusal, not a fact about the world, and hiding it would let an automatic link be derived again, against the owner's refusal.
- **Hidden data may suppress, never create (D50, accepted 2026-11-06).** The veto and automatic-link conflicts use every owner intent and every conflict regardless of the reading scope. A globally blocked account stays unlinked in every scope; creating a link requires premises visible in that scope. The accepted disclosure is one bit: an account may stay unlinked without a visible reason.
- The issuer must be able to see the target when the intent is issued; an intent from a context that cannot is refused. The CLI is authorized to see every scope, independently of its recorded issuer scopes (D35 below). The recorded issuing context is audit data, not lasting authorization: an intent issued in one scope applies in another that can see its target (`../decisions/knowledge.md` §42, §44).

## Rebuild, replay and cache

- The intent table sits outside the swapped projection tables (`0009-claims-projection.md`), so a rebuild never discards it and an older binary ignores it.
- A rebuild replays the evidence, carries over the pairs that cannot replay (`../decisions/knowledge.md` §50), then replays the intents, then resolves links and merges, with the cannot-link veto, before publication.
- The intent-backed claims come out identical to the incremental projection: same ids, same order, and the intent table unchanged, rows and order.
- An intent write invalidates the scoped-read cache. Today's cache is keyed by projection high-water marks (`0009-claims-projection.md`, the 2026-11-05 benchmark gate result; `../decisions/knowledge.md` §49) and has no intent watermark.

## Presenting a group

- Equivalence is limited to merged Persons; claims about different accounts held by one Person stay separate (`../decisions/knowledge.md` §59, plan D36). A group of equivalent claims carries its member claim ids, each with its own status and revision; its revision is the hash of the sorted (member id, revision) pairs; it counts as one toward `limit` (`../decisions/knowledge.md` §35).
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

## Build notes confirmed by the owner (2026-11-05)
Confirmed as built (plan D30).

- A Person's name: the newest non-undone `name` intent wins; earlier stretches stay historical. Undoing the newer name reopens the older one from the undo, preserving the gap (D39 below).
- A cannot-link between two accounts (a no to a candidate) also blocks any automatic link that would put both under one Person.
- Draft `domain_key` formats: `person/<id>` (`person/me` reserved), `person/<id>/name`, `held_by/<account>/<person>`, `cannot_link/<a>/<b>`, `same_as/<a>/<b>`.
- Every row is validated atomically: JSON shapes, non-empty ids and kinds, unique set members, a valid anchor, and a hash that agrees with the snapshot.
- The slice-13 export keeps the intents' relative acceptance order.
- A cleared cannot-link stays in `rastro identity history`, marked reversed.

## Amended 2026-11-05 (owner decisions after the build review)
Status stays accepted. Plan: `../plans/2026-11-05-second-source-one-identity.plan.md` (D25–D29, D32, D34–D41). Product meaning in `../decisions/vocabulary.md` §27 and `../decisions/knowledge.md` §58.
- **Only an account is held by a Person (D25, extends *Core identity predicates*).** The core declares the supertype `rastro:Account`; a vocabulary declares which of its types are accounts (`code:Account`, `tracker:Account`). A new link, by the owner or automatic, whose account side is not typed as an account is refused, as a `held_by` from a Person already is. Replaying an existing owner link preserves it (D41 below).
- **Undone and older claims stay `superseded` (D26, extends *Resolution* and *Intent-backed claims*).** An undone link or merge, and a Person's older name, stay in the projection as `superseded`, with the interval closed at the undo (or the newer name's) time. An older name still resolves as a historical alias.
- **No undo of `create_person` (D27, extends *Resolution*).** An `undo` of a `create_person` intent is refused until slice 14's controlled removal. A mistaken Person is merged into the right one.
- **`issuer_context` records real scopes (D28, extends *The owner-intent table*).** It holds the issuing surface's actual configured scopes, never `[]` standing for all. Still audit data, not authorization.
- **A retried `person create` mints a new Person (D29, extends *Minting a Person*).** Each run of `rastro identity person create` is a new intent with a new id; a duplicate is merged. Revisit when an agent surface can issue intents.
- **A Linear guest's email is not verified (D32, extends *Automatic links*).** Only a workspace member's email is a verified email; a guest's never makes an automatic link, though it may still feed a fuzzy candidate. Plan D32 (owner Q11).
- **The token owner is the `viewer` item (D34, extends *Automatic links*).** Each connector records its token's owner as kind `viewer` at `<namespace>:viewer`, field `/id`, and an extractor emits `rastro:token_owner` from this evidence; the core derives the link from that claim (D44). Product meaning in `../decisions/plugins.md` §25.
- **Issuer scopes are audit only (D35, clarifies D28 and *Visibility*).** The CLI is authorized to see every scope. The recorded list contains only configured sources' scopes, excluding storage-only scopes; `[]` means no configured sources or a legacy row written before D28, never all scopes. A scoped read shows only issuer scopes its reader can see.
- **A merge does not reveal a hidden Person (D37, clarifies *Visibility*).** A Person with no visible account of its own stays hidden even when merged with a visible Person; `me` keeps its every-scope visibility. Traversal in a scope follows only merges whose two Persons are visible.
- **The accounts list shows current links (D38).** An undone link appears as a `superseded` `held_by` claim under `--history`, naming its account, rather than as a current account.
- **Undoing a newer name preserves the gap (D39, clarifies *Resolution* and D26).** Name A at t1, B at t2, undo B at t3 gives A [t1,t2) and [t3,∞). Intent contributions are stored as stretches with `started_at` and `ended_at`; undo opens a new stretch instead of rewriting known-time history.
- **Replay preserves existing owner links (D41, clarifies D25 and *Rebuild, replay and cache*).** The account-type check runs when a link is issued, not when that intent replays. A link made before a type change, or before D25, survives even if its account side is no longer typed as an account. A later slice may flag a held non-account in the Person view.
- **Upgrading leaves an account-type gap until replay (D40).** Linking an existing account is refused until `rastro sync` or `rastro rebuild` replays the vocabularies declaring `rastro:Account`. The error and release notes explain this requirement; existing links are unaffected. No migration fills the supertype, because the core may not name plugin vocabularies.
