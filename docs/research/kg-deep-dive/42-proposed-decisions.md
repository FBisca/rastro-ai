# 42: Proposed decisions (knowledge graph deep dive, step 5)

Agent: `kg-decider`, 2026-11-05. **Proposals only.** The owner confirms or changes the whole table. C1–C18 (`41`) are accepted and treated as fixed. Nothing here is written to `docs/decisions/` or `docs/adr/` yet.

Refs: `40 §` = decision brief; `30 F#` / `31 F#` = review findings; `20`/`21` = the two designs; `00` = current state. T4/T5 = worktrees `../rastro-ai.worktrees/claims-t4` (`4ac017f`), `claims-t5` (`7a532bb`).

**Stance behind every choice.** The next slice is observed-only (knowledge §23). Decide the product semantics of later slices now, so the schema stays open. Build only what observed-only needs. Anything that can be added by a later additive migration is not built now. Where an option needs a premature table, a draft standard, or a core rule that a plugin should own, it loses.

Facts re-checked in the worktrees: T5 `vocabulary.toml` declares `cardinality = "one"` (lines 69, 83, 97); neither SDK has `deny_unknown_fields`; `claims.rs:288` break is real per `31`; main already ships migration `202611061000_claims_projection.sql`, so the new migration must come after it and drop its tables (they are disposable, ADR 0009).

---

## 1. Summary table

| # | Decision (short) | Chosen option (with sub-choices) | One-line why |
|---|---|---|---|
| D1 | What is precious | **1d**: observed-only now, no artifact tables. Record direction: at slice 5, artifacts are precious (1a) unless slice 5 shows 1b suffices; no model call at rebuild (C17). Sub: (i) revisions use a portable evidence key, no local ids; (ii)–(iii) entry conditions for slice 5 (forget path + insert-only triggers in the creating migration; request id ≠ execution id); (iv) vocabulary change = deterministic replay now; (v) decided at slice 5 | No consumer yet; artifact tables are additive later; avoids F8/F11 shipping half-specified |
| D2 | Confirmation, retraction, undo | **Mix of 2b/2c**: bound to the *lifecycle state the owner saw* (occurrence + conflict/supersession links), not the full revision. Retract lapses to a flag + Ask when that state changes. Confirmation stays live but goes *stale* (no Act basis) on change. A new yes on a retracted claim = explicit reverse. Intents apply in every scope that can see a support of the target; issuer must see the target. Precedence: retract > confirmation > deterministic observed > hypothesis > superseded. Anchor reject retracts anchor-conditioned slots. Merge undo always records cannot-link. Carry-through: no Action runs until a durable outbox exists (gate on slice 8) | Fixes all four 30 F1 scenarios without a permanent block, no per-corroboration Asks |
| D3 | Justification structure | **3a**: derivation = AND set of evidence spans + AND set of premise *claims*; OR only between derivations. Id includes claim + extractor + derivation. Cycle rule over the whole dependency graph, anchor exempt. Scalar slot includes subject + predicate. Span table now; premise table with the first inference | Fixes 31 F1/F2, 30 F5.1; T5 author/alias need span bundles today |
| D4 | Domain inference | **4d** now (no inference). Direction for slice 5: **4b** (derivation plugin, core validates premises); no fixed core rule registry | Observed-only needs none; avoids the concept tension of 31 F13 |
| D5 | Equal timestamps, occurrence shape | **5a**: occurrence ids; transitions ordered by event key (`captured_at`, portable evidence key; local evidence id only as the local tie-break); recurrence kept at equal t; zero-length allowed. Granularity: **occurrence rows per justification**, claim presence computed on event order | Fixes 31 F7 / 30 F5.2–3; keeps O↔C acyclic (C5) |
| D5b | "As of" and audit | **5b-a**: capture-cutoff reconstruction under today's rules, labelled as such; one insert-only publication log row per publication (versions, high-water) | Cheap; matches knowledge §25; no receipts until someone needs them |
| D6 | Merge representation | **6a semantics, delivered in slice 4**: overlay, immutable ids, grouped result by default once a merge exists (no change in slice 3). Group = member list + fingerprint (sorted member ids+revisions); no representative id; confirmation targets member claims atomically. Glossary: **Merge = same-kind entities are one; account→Person is a link claim, not a merge**. Undo = cannot-link | "One identity per thing" (concept §1) forbids duplicate cards by default; no cost before slice 4 |
| D7 | Person minting | **7b**: Person ids never derived from email. Minted from owner intent, address-book resource id, or an owner Rule (a precious input). Email/account links are time-bounded claims; equal vouched email *justifies* a reversible `held_by` link to an existing Person or proposes one; overlapping equal emails only. Exact formula in the slice-4 ADR | Fixes 31 F10 (reused work email); keeps undo possible |
| D8 | Several extractors, retirement | **8b + retirement rule**: one binding per source now; slot contract only when a second extractor exists. On a version bump the new version replays the pair, and the old version's justifications are replaced **atomically when the replay completes** (reads serve the old output until then; failed upgrade keeps last good). Several extractors = same claim, several justifications, each labelled with its extractor; not independent corroboration | Closes the rebuild≠incremental gap (00 §5 A8) without building multi-binding |
| D9 | Scope strategy | **9c**: read-time exact recompute from visible justifications + intents; stored full-scope state is an acceleration for unscoped reads only; adjacency-style index and the scoped sweep input in the schema; writer runs `ANALYZE` at publication; benchmark gate before T8 ships; lenses only if the gate fails. Required: random scope-set oracle with hidden-input invariance | Correct by construction, no invalidation burden, small |
| D10 | Revision inputs | **10a content, 10d computation**: visible state only (status, visible support portable keys, links, validity) hashed as a sorted, deduplicated set; no versions, no local ids; new tag `rastro/revision/v2`. Renderer stale-check still uses this revision; the durable intent binding (D2) uses the lifecycle fingerprint | Fixes 30 F4, 31 F15; stops vocabulary typos expiring ticks |
| D11 | Standards and ids | **11c now, with one cheap re-version**: no RDF/PROV/nanopub profile, no IRIs. Core reserves the `rastro:` prefix; T5 meta-claims become `rastro:type`, `rastro:alias` (ids change, nothing stored). Entity/claim id encodings stay v1. Lexical datatype identity (no canonicalisation). Namespace authority for plugin prefixes decided at slice 16. Direction: JSON-LD 1.1 statement-node serializer at the edge when Share is built | No consumer, drafts moving (30 F6); renaming is free now |
| D12 | Vocabulary validation | **12a**: fixed Rust checks + conformance fixtures; bundle-atomic type grounding (a bundle carries the types of its own endpoints); `deny_unknown_fields`; `cardinality` required; optional `supertype` | One mechanism, no CI SHACL engine (30 F9) |
| D13 | Agent query surface | **13b**: fixed operations now; bounded expansion only after an equal-token evaluation beats FTS. Bare name stays an operation error (no amendment). No owner ad-hoc query surface now | Smallest surface; budgets unmeasured |
| D14 | Schema, migration, publication | **14a + 14c**: one new migration `knowledge_v2` replacing main's T3 projection tables, observed-only (about 8 tables, no intents/artifacts/merge/premise/valid-time/lens tables). **P1** (TEMP shadow swap, explicit TEMP DDL). Valid time reported as `unknown`, no column. `ANALYZE` at publication. Insert-only triggers in the migration that creates each precious table. `Source::github` hard-code removed | Migrations are never edited, so ship only what is needed |
| D15 | T4/T5 fate | **15a**: rework both onto the one migration (T4′, T5′), merge neither. Paused plan edited, not replaced | Little code survives, but semantics, fixtures, rules and extractor do |

---

## 2. Per decision

### D1. What is precious

**Chosen: 1d, with a recorded direction.**
- No artifact, intent or rule tables in the next migration. Rebuild = replay evidence through deterministic extractors, as today.
- Recorded direction for slice 5: model outputs are **recorded, replayed, never re-called at rebuild** (C17). Whether the recording is precious-and-exported (1a) or only the owner's confirmed interpretations are kept (1b) is decided when slice 5 is planned. The owner should expect 1a: 1c is rejected now, as it breaks "knowledge = f(evidence, confirmations, rules)".
- Sub (i) **portable evidence key: yes.** `evidence_key = hash(source, scope, location, field, start, end, quote, captured_at)`, computed at projection time and stored on the justification span. Revisions and occurrence ids use it, never the local integer id. Cross-machine tie-breaks at equal `captured_at` are left to slice 13: import preserves export order, so local id order stays the order.
- Sub (ii)–(iii) are **entry conditions for slice 5's migration**: insert-only triggers and a forget path for every copied payload in the migration that creates the table; request identity separate from execution identity; activation/retirement of the older execution on a successful re-extract.
- Sub (iv): vocabulary definition change now = deterministic replay (free). For hypotheses: re-validate and mark `behind`, do not re-call the model (decided at slice 5).
- Sub (v): decided at slice 5. Leaning: structured and unstructured may back a hypothesis; untrusted content only through an owner Rule.

**Why not the others**
- 1a now: artifact growth unmeasured (40 §D1); forget/export obligations (30 F8.2, 31 F11) would ship half-specified in a migration that can never be edited. Tables are purely additive later, so deferring costs nothing.
- 1b now: same tables needed, weaker guarantee, no consumer.
- 1c: "if deleting it changes knowledge it is not a cache" (40 §D1); contradicts C17.

**Reviewer scenarios.** 30 F8.1 (local ids in revisions) fixed now via the portable key. 30 F8.2–3, 31 F11: no artifacts exist, so no exposure; entry conditions above are binding on the slice-5 plan.
**Risks accepted.** Slice 5 may need a migration plus a short design pass; hypotheses stay blocked until then (already true).
**Amends.** Knowledge §12 gets a dated note ("slice 5 records, not re-calls"); knowledge §12 text itself changes only when slice 5 picks 1a. No glossary change (do not add "extraction artifact" yet).

---

### D2. Confirmation, retraction and undo

**Chosen (product semantics, built in slice 7; schema-neutral now).**
- **Binding.** An owner intent on a claim binds to the **lifecycle state the owner saw**: the claim's current occurrence plus its conflict/supersession links. Not the full ADR 0009 revision (extra corroboration must not expire a yes), and not the whole proposition forever.
  - **Retract/reject** hides the statement while that state holds. If the state changes (the occurrence closes, it is superseded, reopens after a gap, or gains a conflict), the retract **lapses**: the claim reappears with its evidence-derived status, marked "earlier retracted", and an Ask is raised.
  - **Confirmation** keeps the statement alive on its own (C12; survives provenance change and extractor retirement). If the state changes it goes **stale**: shown with its history, no longer deterministic evidence, so no Act (concept §2; glossary "Deterministic evidence"). An Ask is raised.
  - **A new yes on a retracted claim** is an explicit reverse (latest owner intent wins, recorded). Not a no-op.
  - **Undo of a yes** removes only the yes.
  - A truly permanent block ("never show me this again") is an owner **Rule** (glossary), not an intent.
- **Renderer stale check** is unchanged (renderers §5): the intent carries claim id + full revision seen and a changed revision shows a conflict. The durable binding above is separate.
- **Scope.** An intent applies in **every scope that can see a provenance support of its target**. The issuer must be able to see the target; an undo from a context that cannot see it is refused. This makes hidden intents invisible to other lenses (hidden-input invariance, D9) and stops a work-scoped agent acting on something the owner retracted in personal when it could see the claim.
- **Precedence** (one order, both designs agree): valid retract > valid confirmation > deterministic observed (including a rule over observed/confirmed premises, C14) > hypothesis > superseded.
- **Anchor and bundle.** A confirmation stores the exact reviewed claim set (C12) with the anchor identified. Rejecting the anchor retracts the slots whose only justification is conditioned on it; a slot with an independent observed justification survives (C7). The owner can still retract slots one by one. The anchor is never its own premise.
- **Undo of a merge** always records a cannot-link (C4). One operation, no "undo vs they are different" chooser.
- **Carry-through delivery.** Documented interim boundary: until a durable outbox exists (written in the same transaction as the intent or projection publication, idempotent consumers, proposals re-evaluated against OR alternatives rather than blanket-withdrawn), **slice 8 may not run an Action on a knowledge-derived basis**. Slices 6–7 emit events best effort; nothing executes on them.

**Why this over the alternatives**
- 2a (proposition-bound): permanent block outlives its world state (30 F1.1, brief §D2). Rejected.
- 2b (full revision): corroboration or a vocabulary edit would re-raise Asks; revision also changes on any second support (ADR 0009). The lifecycle fingerprint keeps 2b's "new evidence re-raises an Ask" without that noise.
- 2c (occurrence only): close to the choice, but ignores a conflict appearing inside one occurrence; the fingerprint adds links.
- 2d: owner re-confirms after every provenance change; defeats C12.
- Scope "Ask's context only" (20, 21): leaves the claim usable in another scope after the owner retracted it (30 F1.3). "All scopes by default" (30) would let a hidden intent change an answer in a scope that cannot see the target (31 F4). "Scopes that see a support" satisfies both.

**Reviewer scenarios.** 30 F1.1 (retract then true): lapses on the new occurrence. F1.2 (confirm then newer): stale + Ask, not an Act basis. F1.3: covered by scope rule. F1.4: explicit reverse. 31 F3: anchor exempt, bundle persisted, whole-graph cycle check (D3). 31 F4: same-visibility rule; type grounding in-bundle (D12). 31 F12: gate on slice 8.
**Risks accepted.** A retract can lapse more than an owner expects: they get an Ask, never a silent un-hide. Fingerprint definition is finalised in the slice-7 ADR (needs D5 occurrence ids, available).
**Amends.** Knowledge: new entries (binding, precedence, scope of intent, carry-through gate); §26 sharpened to place confirmation in the precedence. No concept edit (concept §1 "correctable" holds: the owner can retract anything; a permanent block exists as a Rule). Glossary: no new Intent kinds needed. 20's `retract` = `reject`; `undo_confirmation`/`block_merge` = `undo`; `reverse_rule` = `confirm` on a retracted claim (30 F12 resolved by mapping, not by new terms).

---

### D3. Justification structure

**Chosen: 3a.**
- A **derivation** (one justification) holds an AND set of evidence spans and, once inference exists, an AND set of premise **claims**. OR only between derivations of the same claim.
- A premise is live while **any** justification of the premise claim is live (30 F5.1).
- Justification identity = hash(claim id, extractor id, derivation key). Scalar slot = (source, scope, location, field, subject, predicate); the **field group** for cross-extractor conflict is the slot without the rule (T4 `lifecycle.rs:24–40` already does this).
- Cycle rule: reject cycles over the **whole** dependency graph (provenance, premises, confirmations); an anchor is exempt from its own premise.
- Tables now: `justification` + `justification_span`. The premise table arrives with the first inference (additive). `kind` column accepts the three kinds from day one (only provenance used).

**Why**
- 3c (one span per row) turns `/user/id` AND `/user/type` into two OR proofs: forgetting one leaves authorship held (31 F2). T5 emits exactly that today.
- 3b (premise names an occurrence) kills an inference while the claim lives (30 F5.1).
- 31 F1 (`repository-from-url` emits two conclusions from the same span): claim id in the justification identity and subject+predicate in the slot stop the unique-key failure and the mutual replacement.

**Risks accepted.** Child span table; liveness = every span live (a justification dies if any required span is forgotten). Defined in the ADR.
**Amends.** ADR 0009 §Tables (rename `claim support` → `justification`, add span child); ADR 0008 §Core validation (a submission's supports are an AND bundle). Knowledge §5/§6/§14 notes: forgetting any span of an AND bundle kills that derivation; "two supports" means two derivations.

---

### D4. Domain inference

**Chosen: 4d now; direction 4b.**
- No inference in the next slice. The first slices are observed-only (C14 stays for later).
- Direction recorded for slice 5: derivation (e.g. "lift a promise to the Person", "resolve 'by Friday'") is a **replaceable plugin** submitting claims with declared premises; the core does generic validation and maintenance only (31 F13). No fixed core rule registry.

**Why**
- 4a (core registry): domain rules in core contradict concept §4 / vocabulary §1 and ADR 0008's rejection of an in-core mapper; the owner would fork for a different calendar.
- 4c (owner rule language): a language the core must own; no need yet.
- Identity components (merge connectivity) are non-inferential core computation (20 A9), not domain inference; unaffected.

**Risks accepted.** Slice 5 needs a protocol message for derivation first. Better than shipping core rules to remove later.
**Amends.** Nothing now. Slice 5: ADR 0008 amendment, plugins §2 (derivation as an extractor role, not a new kind, to avoid a glossary term).

---

### D5. Equal timestamps and occurrence shape

**Chosen: 5a, per-justification occurrence rows.**
- Transitions are ordered by **event key** = (`captured_at`, portable evidence key); local evidence id is only the local tie-break. Intents use an explicit intent sequence.
- Occurrence id = hash(justification id, opening event key). Minting inputs are exactly those, so rebuilds reproduce them (30 F5.3).
- Known intervals are half-open and may be zero-length; a closed occurrence and a later recurrence are separate rows even at equal `captured_at`.
- Claim presence is the union of its justifications' occurrences **on event order, not timestamp**: `[a,t)` closed at event 2 and reopened at event 3 are two occurrences.
- `superseded_by` = the current transition only (C5).

**Why**
- 5b (coalesce): loses intermediate states and still needs rules for equal t (31 "Questions"); D2's binding needs occurrences.
- 5c: duplicate PK in SQLite (reproduced in 31 F7).
- Per-lens claim intervals (21) need lenses; per-justification rows give scoped recompute for free (D9).

**Reviewer scenarios.** 31 F7 (O, C, O at equal t): three distinct occurrences, no PK clash. 30 F5.2 (union erases recurrence): explicit event-order rule.
**Risks accepted.** More rows than coalescing; recurrence at identical t is rare, kept anyway.
**Amends.** ADR 0009 §Tables (occurrence-level), §Replay order (event key). Knowledge §25/§26 sharpened. "Occurrence" stays an implementation term.

---

### D5b. "As of" and the served-answer audit

**Chosen: 5b-a.** "As of t" = recompute from evidence captured by t under **today's** pinned rules, labelled as reconstruction. One insert-only `projection_publication` row per publication (extractor and vocabulary versions, evidence high-water) answers "which rules produced this generation". Outside the swapped tables so a rebuild does not discard it.

**Why**
- 5b-b (answer receipts): new precious data and retention policy, no consumer before slice 7.
- 5b-c (contextual ids): rewrites ADR 0009 §Ids and renderer targets.
- D8's retirement means the narrow promise is the only honest one.

**Risks accepted.** "Why did you say X on Tuesday?" gets today's rules on Tuesday's evidence. State it in the answer. Revisit if the owner needs the exact served answer (act records, concept §7, are separate: an Action run already records its justification).
**Amends.** Nothing in concept; ADR 0009 gains the log row.

---

### D6. Identity: merge representation and the word "Merge"

**Chosen: 6a semantics, delivered with slice 4.**
- Merge is an overlay claim; claim ids are never re-keyed (C2, C4). No merge machinery in the next slice.
- Once a merge exists, `about` **groups equivalent claims by default** (concept §1 "one identity per thing"): a group carries all member claim ids and their own statuses/revisions; group revision = hash of sorted (member id, revision); a group counts as one toward `limit`. No representative id in the contract. The group fingerprint is never a claim id. A confirmation targets the member claims atomically; a change to any member shows as a conflict (31 F9). Slice 3 has no merges, so D15 ordering is untouched.
- Glossary: **Merge = a claim that two entities *of the same kind* are one. An account is linked to a Person by a `rastro:held_by` claim; it is not merged.** Accounts are not humans (C3, vocabulary §12). No new term "link".
- Undo records a cannot-link (D2).

**Why**
- 6b (opt-in grouping): leaves duplicates on the default card, against concept §1; also leaves 31 F9's targeting schema undefined. Since grouping is a no-op in slice 3, deciding it now is free.
- 6c (re-key + redirects): breaks renderer targets and C2.
- 6d: stalls slice 4.

**Reviewer scenarios.** 31 F9 (P1 confirmed, P2 changes): the group revision changes, so the intent shows a conflict. 30 F10.1 (min-member flips): no representative id. 30 F10.2: limit and revision rules above.
**Risks accepted.** Group status presentation (mixed members) finalised in the slice-4 ADR.
**Amends.** Knowledge §5 clarified ("one claim per exact statement; equivalents grouped on read"); renderers §5 (target sets); surfaces (result shape, slice 4); **glossary Merge row** (see §4).

---

### D7. Person minting and email durability

**Chosen: 7b, with constraints; the formula is set in the slice-4 ADR.**
- A Person id is **never derived from email text**.
- A Person exists through: an owner intent (an accepted Ask), an address-book resource id, or an owner **Rule** (glossary; a precious input, so automation is replayable and switch-off-able).
- Account→Person and email links are **time-bounded claims**. An equal vouched email *justifies* a reversible `held_by` link to an existing Person, or proposes one (Ask); equal emails over **non-overlapping** intervals never join Persons (Ana in January, Bea in June). Two Persons are never joined by email alone.
- No provider normalisation (Gmail dots) baked into identity.

**Why**
- 7a bakes email reuse into identity before the reversible layer (31 F10), so rejecting a link cannot split two humans.
- 7c needs a split mechanism; 7b gets the same result with links.
- Knowledge §4 ("exact identifiers merge automatically") is honoured by the Rule path.

**Risks accepted.** Fewer automatic joins until a Rule or address book exists. Interim: Asks.
**Amends.** Knowledge §24 not extended. Vocabulary §1/§12: core predicates `rastro:held_by`, `rastro:same_as` named in the slice-4 ADR.

---

### D8. Several extractors and version retirement

**Chosen.**
- **One binding per source** (8b); the slot/replacement contract waits for a second extractor.
- **Retirement rule now:** a version bump replays the pair into new-version justifications; when that replay completes successfully, the old version's justifications are replaced **in the same transaction as the final batch**. Reads serve the old output until then. A failed upgrade leaves the last good output. Rebuild = incremental (00 §5 A8 closed).
- Several extractors on one field group: **one claim, several justifications, each labelled by extractor**. Knowledge §5 stands, but corroboration is not implied by count; `why` names extractors and sources separately. Authority across extractors: the field group excludes the rule (T4 already). Owner override is a Rule, later.

**Why**
- 8d (keep old supports): rebuild ≠ incremental; stale interpretations stay current.
- 8c (delete on catch-up): this choice is its atomic form, avoiding a union window.
- 8a (multi-binding now): slot declarations for a case that does not exist (30 F12).

**Reviewer scenarios.** 31 F15 (h→2h double count): old and new never coexist in a visible state, and D10's set-dedup removes multiplicity anyway.
**Risks accepted.** Old-version history is not kept as interpretation; evidence and replay remain (D5b limit).
**Amends.** ADR 0008 §Binding (retirement rule); plugins §15 note.

---

### D9. Scope strategy

**Chosen: 9c.**
- **Exact recompute at read time** from visible justifications and visible intents. Unscoped reads may use stored full-scope state (acceleration only; oracle is the recompute).
- Schema carries what the scoped sweep needs: on each justification occurrence, (source, scope, location, field, captured_at, source_updated_at, portable key), indexed; plus an index so `about` can read a subject's/object's claims by recency without scanning the whole hub.
- The **writer runs `ANALYZE` at publication** (a read-only open cannot, ADR 0002).
- **Gate before T8 is done:** benchmark the real scoped sweep (not the join) on a 20k-claim hub and a 100k-claim DB, cold and warm, with two scope sets; target p95 < 200 ms. If it fails, add a cache for common launch scope sets, keep exact recompute as oracle and fallback, and fail honestly on a work budget instead of returning a weaker answer (31 F5). Lenses are not designed now.
- **Required tests (any option):** random scope **sets**, hidden-input addition/removal invariance (evidence, intents, later artifacts/rules), per-scope rebuild equals scoped read. T4's `refresh` is the naive oracle for observed-only only.

**Why**
- 9b (lenses): storage Σ over registered sets (31 F5: 512 copies), invalidation across merges, registration at launch breaks the read path (ADR 0002), GC of a long-lived MCP session. Heavy for a DB with 1,520 evidence rows (00 §4).
- 9a (pure read-time): 30 F2's 103 s without `ANALYZE`; fixed by `ANALYZE` + schema-carried sweep input + the gate.
- 9d: leaks (knowledge §14/§26).

**Reviewer scenarios.** 30 F2 (plan fragility, sweep unspecified): `ANALYZE`, sweep input in schema, gate benchmarks the sweep. 31 F4 (hidden base state): intents follow D2's visibility rule; type grounding in-bundle (D12). 31 F6: set-based oracle.
**Risks accepted.** Cost O(degree) for a scoped call on a hub until the gate says otherwise; scale numbers are unmeasured proposals (40 §4).
**Amends.** ADR 0009 §Scope (mechanism stated). No new ADR now.

---

### D10. Revision hash inputs

**Chosen: 10a content, 10d computation.**
- Revision = hash of: status, the **sorted, deduplicated set of visible support portable keys** (not local ids, not rule ids, not versions), conflict/supersession links (claim ids), validity. New tag `rastro/revision/v2`. Versions stay on the justification for `why`.
- O(n) per recompute; optimise only after profiling (a claim with 10k supports is not expected soon).
- Renderers' stale check (renderers §5) keeps using this revision; D2's durable binding uses the lifecycle fingerprint.
- T4 changed the encoding under the v1 tag (`claims.rs:110`); v2 repairs that. Entity and claim id v1 encodings stay.

**Why**
- 10b (hashes of definitions/versions): a `definition` typo expires every pending tick (30 F4).
- 10c (sum of per-support keys): multiplicity during upgrades (31 F15); collision proof outstanding (40 §4).

**Risks accepted.** Cannot tell "same state, new rules" from the revision; `why` and the publication log can.
**Amends.** ADR 0009 §Ids and revision.

---

### D11. Standards profile and ids

**Chosen: 11c now, plus one cheap re-version.**
- No RDF/PROV/nanopub profile, no IRIs, no export of claims. Export is precious inputs (sharing §1); claims are not exported.
- **Re-version once, now:** the core owns the `rastro:` prefix. T5's meta-claims become `rastro:type`, `rastro:alias` (a datatype `rastro:type` too). Their claim ids change; nothing is stored (00 §4). Entity id and claim id encodings stay v1 (golden tests keep passing); revision becomes v2 (D10).
- Datatypes keep **lexical identity** (`"01"` ≠ `"1"`; `date` ≠ `datetime`); canonical forms per datatype can be added later under an explicit id version (31 F14).
- Plugin prefix authority (two `code` vocabularies colliding at Share) is decided at slice 16; Share must disambiguate by vocabulary id + version.
- **Entry conditions for slice 16** (recorded in the sharing decision): hypotheses map to *unasserted* statements; round-trip tests check proposition identity and status, not counts; a statement-node JSON-LD 1.1 serializer at the edge.

**Why**
- 11a (publish a profile now): tracks drafts (RDF 1.2 CR Snapshot, SHACL/SPARQL 1.2 WDs; 30 F6), no consumer in the roadmap, IRIs leak into every key.
- 11b: its useful part (re-version once) is adopted; the IRI scheme is not (30 F7.2 authority problem).
- 11d/11e: forgo inspectability / scope now.

**Reviewer scenarios.** 31 F14: no export exists to mislead; entry conditions stop it at Share. 30 F7: no legacy adapter, no IRI authority.
**Risks accepted.** Interop later needs a mapping; other installs with ids stored under old names (check `git tag --contains` for `202611061000`, 40 §4).
**Amends.** ADR 0009 §Ids (revision v2, reserved prefix); ADR 0008 (meta-predicates); vocabulary §1 (core meta-model predicates); sharing (entry conditions).

---

### D12. Vocabulary model and validation

**Chosen: 12a.**
- Fixed Rust checks plus conformance fixtures; no SHACL-style compiler, no SHACL export until a consumer exists.
- **Bundle-atomic type grounding:** a submission's claims are validated together; endpoint types come from type claims **in the same bundle**, not from stored base state (replaces T5's "type claims first" ordering; avoids 31 F4's hidden-type dependency). A conflicting type from another bundle is a visible conflict, with `supertype` compatibility (C9).
- Vocabulary parsing: `deny_unknown_fields`; `cardinality` required (C11); optional `supertype`; cross-namespace `dependencies` as today.
- Overlap mappings remain confirmed claims (vocabulary §7). Definition change = replay (D1 iv).

**Why**
- 12b: second mechanism covering a subset, needs a SHACL engine in CI that is not identified, `domain_json` not relational (30 F9).
- 12c/12d: size or no interop; neither needed.

**Risks accepted.** Every new vocabulary feature is core code (small, rare).
**Amends.** ADR 0008 §Core validation; vocabulary §§1, 3, 4, 13 (cardinality required, supertype).

---

### D13. Agent query surface

**Chosen: 13b.** Fixed operations only: `search`, `coverage`, `about` (one hop, D15 order, one-hop identity grouping from D6 when slice 4 lands), `why`. Bounded expansion is added only after an equal-token evaluation on 40–60 owner questions beats FTS (14 "Evaluation"). Budgets (5 seeds, depth ≤2, 25 neighbours, 2,000 examined, 50 out, 64 KiB) are placeholders until measured. **Bare name stays an operation error** (surfaces §23); no amendment. **No owner ad-hoc query surface now**; revisit with sharing/export.

**Why**
- 13c: more surface now; 31 F8 shows `LIMIT` is not a work bound without an index including `live`; hub cut-offs unmeasured.
- 13d: unbounded work, no row-level scope joins; C16.
- 13a differs only in the evaluation gate; keeping the gate costs nothing.

**Risks accepted.** Several round trips; no "count all X".
**Amends.** Nothing now; later additive parameters amend ADR 0007/surfaces.

---

### D14. Schema, migration staging, publication

**Chosen: 14a + 14c, publication P1.**
- **One new migration `knowledge_v2`**, dated after `202611061000`. It drops main's disposable T3 projection tables and creates the observed-only set, about eight: `entity` (id, canonical identifier; **no type column**, types are claims), `claim`, `justification`, `justification_span`, `occurrence`, `claim_link`, `extraction_checkpoint`, `projection_publication` (log, outside the swap). Aliases are alias claims with an index on (`rastro:alias`, value); no `alias` table. T4's migration is never applied.
- **Not in this migration:** intents/owner events, rules, artifacts, merge/cluster, premise, valid-time, lens, vocabulary tables. Each arrives in its own additive migration with its own insert-only triggers and forget path (D1).
- **Publication P1:** T4's TEMP shadow swap in one transaction, with **explicit TEMP DDL** instead of stripping `REFERENCES`. Leave P1 when publication > 60 s or WAL > 2× DB (21 R10); then adopt generation keys (P2).
- **Valid time:** reported `unknown` by the API, no column (ADR 0009's "stored as unknown" is reworded).
- **`ANALYZE`** on projection tables at publication (D9).
- **Insert-only triggers** on `projection_publication` now, and on each later precious table in its creating migration.
- **`Source::github` hard-code in rebuild is removed;** the source comes from the `sources` row.

**Why**
- 14b: 19–25 tables for a slice that needs about eight (30 F3); shipped tables are never editable.
- P2: generation prefix on every key and query, collection under a read-only reader unresolved (30 F11); P1 is simpler and has a measured exit.

**Reviewer scenarios.** 30 F3 (split the DDL): done. 30 F11: P1 first. 30 F8.3: triggers rule.
**Risks accepted.** Later migrations must fit what ships. The O(projection) publication copy at current scale (1,520 evidence rows) is fine.
**Amends.** ADR 0009 §Tables, §Writing (publication), §Ids; ADR 0002 migration rule noted (never edit; drop-and-recreate of disposable tables in a later migration).

---

### D15. Fate of T4/T5

**Chosen: 15a.** Rework onto the one migration (T4′, T5′); merge neither as is; the paused plan is edited, its Pause section resolved by this decision.

**Why.** 15b delays observed-only value for a model whose later parts are deferred; 15c does not compile (`claims.rs:288`) and would freeze T4's migration; 15d loses about 2.5k lines of tested lifecycle semantics.

**Risks accepted.** About two reworked tickets before T6–T8 (21's estimate). Re-verify the merged tree builds as the first T4′ step (40 §4; not run by me).
**Amends.** Plan tickets; open-questions entries moved to decisions as each is recorded.

---

## 3. Coherence check

- **One thread: observed-only now, semantics decided for later.** D1, D4, D6, D7, D2 fix product meaning for slices 4–7 and add no tables now; D3, D5, D9, D10, D12, D14 define the observed-only engine so those later pieces are additive.
- **Ids and revisions.** D10's portable keys feed D5's event keys and occurrence ids, D1's replay, and D2's binding. No local ids in any hash. Nothing depends on lenses, so D9's read-time recompute can reuse the same per-justification occurrence rows (D5).
- **Intents and scope.** D2's visibility rule ("scopes that see a support") is what makes D9's hidden-input oracle pass; D12's in-bundle grounding removes the other hidden-base-state path (31 F4).
- **Justifications and forgetting.** D3's AND bundles plus D1's no-artifacts mean forgetting one span kills exactly the derivations that need it.
- **D8 + D10 + D5b.** Atomic retirement means one interpretation visible at a time (no multiplicity), revisions ignore versions, and "as of" is stated as reconstruction.
- **D6 + D7 + D11.** Person ids come from intents, address-book ids or Rules, never email; identity predicates are `rastro:`-namespaced; the `held_by` link is time-bounded. Consistent with C3/C4/C9/C10.
- **Tensions left.**
  1. `projection_publication` (D5b/D14) is the one non-disposable table in an otherwise disposable migration. Small; flagged for the owner.
  2. D2's lifecycle fingerprint and the renderer's full revision are two different notions. Both are documented; the slice-7 ADR must define the fingerprint exactly.
  3. D6 default grouping lands in slice 4 and changes `about` output shape then (additive, but flagged).
  4. D9's gate is a promise to measure; scale numbers remain unmeasured (40 §4).
  5. Renaming `type`/`alias` to `rastro:` assumes no other install stored ids under the old names (check).
  6. D2's carry-through gate (no Acts before an outbox) constrains slice 8's plan.

## 4. Concept or glossary edits needed

**Concept: none.** Checked against: §1 cited (spans bundle, no span no claim), time-aware (occurrences), one identity (grouping by default), honest about gaps (unchanged), correctable (retract lapses visibly, never silently; permanent block as Rule); §2 (stale confirmation cannot back an Act); §4 (domain inference not in core; DETERMINISTIC extractors remain plugins).

**Glossary: one edit (D6).** Replace the **Merge** row:

| Term | Meaning | Not to confuse with |
|---|---|---|
| **Merge** | A claim that two entities of the same kind are one. Reversible. An account is linked to a Person by a claim, never merged into it | |

No other glossary change: "extraction artifact", "lens", "occurrence", "slot", "anchor" stay implementation words; no new Intent kinds (D2 mapping above).

**Decision-file entries to write after confirmation** (not part of this task): knowledge (D1 note, D2, D3, D5, D5b, D6, D7, D8, D9), vocabulary (D11, D12), sharing (D11 entry conditions), surfaces (none), plus ADR amendments to 0008 and 0009 as listed per decision.

## 5. Next slice (reworked T4′/T5′, then T6–T9)

1. ADR amendments first: 0009 (tables, occurrence/event key, revision v2, publication log, scope mechanism), 0008 (justification bundles, location context bound, meta-predicates, retirement rule, type grounding, vocabulary format). Decision-file entries dated.
2. One migration `knowledge_v2` (about eight tables, D14); drops main's T3 projection tables; `ANALYZE` at publication.
3. **T4′ lifecycle:** keep T4's lifecycle semantics, strict-newer comparison, conflict/supersession rules, rebuild mechanism and `ClaimView` fields; rework `refresh` into affected-slot recompute with the full replay as test oracle; occurrence rows and event-key order (D5); `rastro/revision/v2` over portable keys (D10).
4. T4′ removes `Source::github` in rebuild; explicit TEMP DDL in the P1 swap; old-version retirement atomic with replay completion (D8).
5. **T5′ extractor:** keep the crate, URL rules, bot handling, alias-quote check and `cardinality` entries; types and aliases become `rastro:type`/`rastro:alias` claims (no in-place `UPDATE entity SET type`); aliases from claims with intervals; retag the seven untagged tests.
6. Vocabulary format: `deny_unknown_fields`, required `cardinality`, optional `supertype`; bundle-atomic type grounding replaces "type claims first".
7. Justification identity includes claim + extractor; AND span bundles for `author` and `alias`; slot includes subject + predicate (fixes 31 F1/F2).
8. Location context documented in the ADR 0008 amendment: same source/scope/location, evidence ≤ batch high-water, recomputable on replay.
9. **T6 `about`/`why`:** D15 order unchanged, no grouping yet (no merges), unscoped reads from stored state; `why` shows justification kind, spans, extractor and version.
10. **T7 failure paths:** pending evidence, crash/retry, `failed`/`behind` states, failed upgrade keeps last good; no artifacts.
11. **T8 MCP + scopes:** scoped read-time recompute from visible justifications; random scope-set oracle with hidden-input invariance; benchmark gate on the real sweep before sign-off; fixed operations only.
12. **T9 sign-off** as planned; owner checks `--reread` data and one scoped answer.
13. Out of this slice: intents, rules, artifacts, merges, Persons, inference, valid time, lenses, standards profile, generation keys.
14. Entry conditions logged for later slices: slice 4 (D6/D7 ADR), slice 5 (D1 sub, D4 direction), slice 7 (D2 fingerprint ADR), slice 8 (carry-through outbox), slice 16 (D11).
15. First action of T4′: build the merged tree at `6358157` in a scratch worktree to confirm the break set before reworking (40 §4).
