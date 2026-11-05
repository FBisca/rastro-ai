# 40: Decision brief (knowledge graph deep dive, step 4)

Agent: `kg-scribe`, 2026-11-05. Neutral: no recommendation of mine. "Leans" are the agents' own, attributed. Nothing here is decided; it lands in `docs/decisions/` or `docs/adr/` only after the owner agrees.

## 1. How to use this brief
- Go in order: each decision (§3) depends only on earlier ones. §2 lists points to accept in one word first.
- Per decision: question, why it matters, options (cost, what each amends, evidence, who leans), depends on / unlocks. Decide, then note the answer in the margin.
- §4 lists unverified facts that could flip a decision. §5 lists the ADRs, decision entries and glossary changes each option triggers.
- Ref legend: `00` current state; `10`–`15` research; `20` = semantic design (agent `arch-semantic`); `21` = pragmatic design (`arch-pragmatic`); `30` = pragmatic review of `20`; `31` = semantic review of `21`. `A#` = area section in 20/21; `Q#` = their owner question; `F#` = review finding. `§` = section; knowledge/vocabulary/etc. = `docs/decisions/<scope>.md`.
- Two corrections to `00 §3` hold across `21` (header), `30 F3` and `31` agreements: T5's `code` vocabulary does declare `cardinality = "one"`; T5's SDK drops it silently (no unknown-field denial). `20 §5` repeated the wrong version. The compile break in the textual merge (`claims.rs:288`) is real.

## 2. Already converged
Both designs and (where noted) both reviews agree. Each is a candidate decision: accept = one word. "Amends" says what changes in the docs.

| # | Candidate decision | Evidence | Amends if accepted |
|---|---|---|---|
| C1 | **SQLite stays the only engine**, behind the `Storage`/`ReadStorage` traits. No embedded graph or RDF engine now; any other engine would be a disposable sidecar, never authoritative. | 20 A7; 21 A7; 12 "Risks"; 31 agreements (SQLite and standard semantics compatible); 30 agreements (Oxigraph rejection). Only `20` also says to correct ADR 0002's engine-date rationale (20 A7). | Nothing (re-affirms ADR 0002). |
| C2 | **Claim ids are content hashes and are never re-keyed** by a merge, an unmerge or a retype. | 20 A4; 21 A4; 13 §9; 30 agreements. | Nothing (keeps ADR 0009 §Ids). Which *encoding* version is open: D11. |
| C3 | **Accounts are not humans. A Person hub:** accounts link to a Person; no literal account = account equality; a shared account may link to two Persons. | 20 A4/§4b; 21 A4/§4b; 13 §6 (P2); 30 agreements. Open: how Persons are minted (D7) and the word "Merge" (D6). | `vocabulary.md` §1/§12 (adds core meta-model predicates, e.g. a "held by" link). |
| C4 | **A merge is an overlay claim, undone without deleting history. No `owl:sameAs`, no OWL equality or functional-property reasoning.** An undo is a durable record that survives rebuild, so a rebuild never re-creates it. | 20 A4/A1; 21 A4 (reject intent as cannot-link); 13 §3–4; 10 "Bottom line". | Nothing new (knowledge §4). Presentation and grouping: D6. |
| C5 | **History is stored per occurrence**, not as accumulated claim-to-claim `supersedes` links (which make open↔closed a cycle). `superseded_by` is the *current* transition only. | 20 A3 + §4a; 21 A3 + §4a; 11 "Concrete example"; 30 agreements. Open: equal timestamps (D5a). | ADR 0009 §Tables (known interval becomes occurrence-level). |
| C6 | **Valid time stays `unknown`**; never derived from an item's update time. | 20 A3; 21 A3; knowledge §25. | Nothing. |
| C7 | **Justification replaces claim support:** kinds provenance, inference, confirmation. Several justifications on a claim are OR alternatives; premises within one are AND. No cyclic inference. No blanket descendant deletion (independent proofs survive). | 20 A2/A9; 21 A2; 11 "Justifications and retraction"; 30 agreements. Open: AND of evidence spans (D3), cycles beyond inference (31 F3). | ADR 0009 §Tables (renames `claim support`). |
| C8 | **Cardinality is report-only.** Two live values for a scalar make a conflict, never a rejection. | 20 A5; 21 A5; vocabulary §13. | Nothing. |
| C9 | **A type is a claim**, not a mutable `entity.type` column; T5's in-place `UPDATE entity SET type` goes. Needs subtyping/compatibility rules so two types are not always a conflict. | 20 A1/A11; 21 A1/A5/A11; 31 agreements. | ADR 0009 §Tables; ADR 0008 (meta-predicates). |
| C10 | **Aliases come from alias claims with intervals.** An old login stays a historical lookup candidate; a reused login is ambiguous and never merges. | 20 §3 (alias notes); 21 `alias_state`; 13 §7. | ADR 0009 §Tables; refines plan S13. |
| C11 | **Vocabulary parsing rejects unknown fields; `cardinality` is explicit** (no `many` default). Fixes the silent drop in T5/T4. | 21 header + A5; 20 A5 (explicit cardinality); 30 F3; 31 agreements. | ADR 0008 (vocabulary format). |
| C12 | **A confirmation stores the full reviewed statement** (not only a claim id), insert-only, outside the rebuildable projection. Not re-checked at rebuild. | 20 §3 `owner_event` + `proposition_bundle`; 21 §3 `intent_target`; knowledge §12. | Nothing (implements §12). Details: D2, D1. |
| C13 | **Location context is needed** (companion fields of a changed record) and must be recorded as an ADR 0008 amendment, bounded, with the delivered ids kept for replay. T5 widened "delivered" without an ADR. | 00 §3 (T5); 20 §5 "Location context"; 21 §5 T5 table. | ADR 0008 §Shape/§Core validation. |
| C14 | **A deterministic rule over observed or confirmed premises can produce an `observed` claim;** inference over a hypothesis stays hypothesis. | 20 A9 "Maintenance"; 21 Q8a + §3 precedence 3–4. Confirmation does not by itself grant an Act (20 §4c step 4). | `knowledge.md` entry (inference statuses). Where such rules live: D4. |
| C15 | **Scoped answers come from visible supports only;** a hidden replacement never hides a visible claim; no new global statistic feeds a scoped answer (FTS rank leakage stays the accepted limit). | 20 A4/A10 + §4d; 21 A6/A10 + §4a/d; knowledge §14/§26, §32. How: D8. | Nothing. |
| C16 | **Agents get fixed catalog operations**, no SQL, SPARQL or Cypher over MCP. | 20 A10; 21 A10; 14 "Agent query surface"; 30/31 do not dispute. Expansion is open: D12. | Nothing. |
| C17 | **Rebuild ≠ re-extract.** Rebuild replays recorded inputs without calling a model. Re-extraction is an explicit action that writes new records. | 20 A9; 21 A9/§3; 15 §9. What is recorded: D1. | Depends on D1. |
| C18 | **Neither T4 nor T5 merges as-is;** both are reworked onto one new migration. The merge does not compile and neither branch has a code review. T4's `refresh` becomes a test oracle, not production code. | 20 A11; 21 A11 + §5; 00 §3; 30 F3; 31 agreements. Shape of the rework: D14–D15. | Nothing. |

## 3. Decision sequence

Order: precious inputs (D1) → confirmation and retraction (D2) → justification structure (D3) → domain inference (D4) → temporal history (D5, D5b) → identity (D6, D7) → several extractors (D8) → scope strategy (D9) → revision inputs (D10) → ids and standards (D11) → vocabulary validation (D12) → agent query surface (D13) → schema and migration (D14) → T4/T5 and next slice (D15). Fifteen decisions (D5 has two parts).

---

### D1. What is precious: do model outputs and owner intents become inputs we must keep?
- **Question:** Besides evidence, confirmations, rules and outcomes, do we also keep the recorded output of any non-deterministic extraction so a rebuild is exact (knowledge §12)?
- **Why it matters:** If the rebuild re-calls a model, claims, ids and revisions drift and the owner's old "yes" no longer matches what exists (15 §9; 21 R7). If artifacts are kept, they copy evidence text, so forgetting must reach them (30 F8.2; 31 F11: "forget one message among several inputs, its words remain in `response`/`submissions`"). Same request with a fresh stochastic output collides on the proposed unique key (31 F11).
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 1a | Artifacts are precious inputs, insert-only, exported, replayed; rebuild never calls a model. | Retention, export, forgetting and portable-id obligations from the first migration; growth per 1k messages unmeasured. | Knowledge §12; sharing §1 (Export). | 20 A9 + Q3a, §3 `artifact`; 21 A9 + Q2a, §3 `extraction_artifact`; 15 §9 | 20, 21, 15 researcher, 31 (with fixes) |
| 1b | Only the owner's confirmed interpretations are kept; unconfirmed hypotheses may change on rebuild. | Weaker replay guarantee, stated openly. | Knowledge §12 (narrower). | 20 Q3b; 30 F8 fix, Q7b | 30 (as alternative until slice 5 shows a need) |
| 1c | Best-effort: pin model/prompt, re-call on rebuild; artifacts are a cache. | Not identical on rebuild; "if deleting it changes knowledge it is not a cache". | None, but breaks ADR 0009 §Replay in spirit. | 20 Q3c; 21 Q2b/c; 15 §9 | None |
| 1d | Observed-only now: no artifact tables in the next migration; decide when slice 5 starts. | Hypothesis path stays blocked; intents tables still needed for slices 4/6/7. | None now; knowledge §23 stays. | 30 F3 (split DDL), 20 §5 migration step 4 | 30 |

- **Sub-questions (apply to 1a/1b):** (i) a portable evidence key in revisions and tie-breaks, else cross-machine replay differs (30 F8.1; verified: `revision` hashes the local `evidence_id`, `crates/rastro-core/src/claims.rs` revision fn); (ii) forget path and insert-only triggers in the first migration that creates them (30 F8.2–3; 31 F11); (iii) separate request identity from execution identity (31 F11); (iv) on a vocabulary *definition* change: re-validate and mark `behind`, or re-extract (21 Q2b, 15 §9); (v) which trust classes can back a hypothesis (21 Q7: structured + unstructured, untrusted only with an owner rule / all / no untrusted).
- **Depends on:** none. **Unlocks:** D2 (what a confirmation can rest on), D5b (audit), D14 (tables), D15.

---

### D2. Confirmation, retraction and undo: what do they bind, for how long, and in which scope?
- **Question:** When the owner says yes, no or undo, what exactly does it attach to (the statement, the revision seen, one occurrence), how does it rank against newer evidence, and who sees it?
- **Why it matters (30 F1):** (1) *Retract, then it becomes true:* owner retracts `PR12 state closed` (stale mirror); PR closes at 12:00 and GitHub says `closed`; a permanent block still hides it, `about` omits it. (2) *Confirm, then newer evidence:* owner confirms `open`, source later says `closed`; the confirmation has no source clock, so it stays `confirmed` beside a conflicting `observed`, and an Act may use the stale confirmed claim. (3) *Retract in personal, claim visible in work:* a work-scoped agent can act on a claim the owner retracted (concept §1 "correctable"). (4) *Yes on a retracted claim* is silently ignored. 31 F3: an anchor confirmation that depends on itself cannot be rebuilt after its extractor is retired. 31 F4: an undo from another scope closes visible support globally.
- **Options (bundles; sub-choices table below):**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 2a | **Proposition-bound.** Yes sustains the exact statement alone, after its provenance changes; a retract blocks it until explicit reversal; undo-yes only removes the yes. Scope = the Ask's context (21: also a conflict flag if later evidence supports a rejected claim). | More retention; a block outlives the world state it described (30 F1.1); needs scope fingerprint. | Concept §1 "correctable" read at scope level (30 F12); possibly a concept note. | 20 A9 + Q4a, §4c; 21 A9 + Q4 (i yes, iii yes + flag) + §3 precedence | 20, 21 |
| 2b | **Revision-bound.** Retract/confirm binds to the revision the owner saw; new structured evidence that changes visible state re-raises an Ask; a newer comparable observation supersedes a confirmed scalar and asks; retract global by default, owner may narrow; a yes on a retracted target is an explicit reverse or an Ask. | More Asks; needs the confirmed-vs-observed comparison rule written. | Knowledge §26 (add confirmation to precedence); renderers §5 (already revision-based). | 30 F1 fix, Q1b, Q2b | 30 |
| 2c | **Occurrence-bound.** A yes binds to one occurrence of the claim; renewal explicit. | A→B→A or any churn expires the yes; renewals. | Knowledge §12/§7 interplay. | 20 Q4c, Q6 | none |
| 2d | **Yes expires whenever provenance changes.** | Owner re-confirms after each source change. | Same as 2c. | 20 Q4b | none |

- **Sub-questions (each can be answered independently of 2a–d):**

| Sub | Options | Evidence / leans |
|---|---|---|
| Scope of an intent | the scope of the context it was given in / every scope by default, owner narrows / owner picks per intent / all scopes that can see a support of the target | 20 Q2 note + 21 Q5a (context); 30 Q2b (all); 21 Q5c; 30 Q2c; 13 §8 (open). Cross-scope undo: refuse or interpret per lens (31 F4, Q "scope isolation"). |
| Precedence among reject / confirmation / deterministic observed / hypothesis / superseded | the 6-row order of 21 §3 step 5, or 20 A9 "Maintenance" order (explicit target block wins, then confirmation, then…) | 21 §3; 20 A9. They differ on whether a block beats an independently sustained confirmation: 20 yes; 21 yes (reject first). |
| Anchor and bundle: reject the anchor of a candidate (e.g. the Commitment) | slots conditioned on the anchor (all retract) / slots independent with a flag / bundle-level retract of exact claims | 21 Q6a/b; 20 §4c step 5; 31 F3 (anchor cannot be its own premise; whole-graph cycle check; persist bundle identity with the intent). |
| Undo of a merge | always records a cannot-link / owner chooses "undo" vs "they are different" | 21 Q12; 20 A4 (block rule until reversal); 13 §4. |
| Carry-through delivery (knowledge §10): flag Action runs, withdraw Proposals | durable outbox + idempotent consumers atomic with the projection / events best-effort (plan D20 defers delivery) / documented interim "no-action" boundary | 31 F12 (crash before delivery leaves the reminder to execute; Proposal with OR alternatives wrongly withdrawn); 20 A9 "Carry-through"; plan D20. |

- **Depends on:** D1. **Unlocks:** D3, D6, D9.

---

### D3. Justification structure: AND bundles of spans, premises, cycles
- **Question:** Is one justification a single evidence span (with AND only across claims), or one derivation with several required spans and premises?
- **Why it matters:** 31 F2: `author(PR, Alice)` needs `/user/id` AND `/user/type`. Stored as two separate justifications they become two OR proofs, so forgetting the id evidence leaves the type-only row supporting authorship (knowledge §6/§14). 31 F1: T5's `repository-from-url` emits two conclusions from the same `/state` span; the proposed unique key `jkey` and scalar slot make the second insert fail or replace the first. 30 F5.1: a premise that names one *occurrence* of one justification kills the inference when that justification closes, though the premise claim lives on through another.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 3a | A derivation holds an AND set of evidence spans plus an AND set of premises; OR only between derivations. Premise = the premise **claim** (live while any justification is live). Identity of a justification includes claim and extractor. | Child span table; liveness and forgetting semantics to define. | ADR 0009 §Tables; ADR 0008 §Core validation. | 31 F2 fix; 20 §3 `support_span`; 30 F5.1 fix | 31 (Semantic side); 20 for spans |
| 3b | Same as 3a but a premise names one **occurrence** of a premise justification. | Dies with the occurrence; contradicts "claim survives any alternative" (30 F5.1). | Same. | 20 §3 `premise` | 20 (as drafted) |
| 3c | One evidence span per justification row; AND only via claim premises. | Cannot express a bundle of spans; T5 author/alias rules need extra claims. | ADR 0009 rename only. | 21 §3 `justification` (one `evidence_id`) | 21 (as drafted) |

- **Also decide:** cycle rule scope: reject cycles over the *whole* dependency graph (confirmation, provenance, premises), exempt an anchor from its own premise (31 F3) vs only over inference rules (21 A2); and justification/slot key must include subject and predicate for scalar slots (31 F1, T4 `lifecycle.rs:24–40` already does).
- **Depends on:** D1, D2. **Unlocks:** D4, D5, D14.

---

### D4. Domain inference: core or plugin?
- **Question:** Rules like "lift an account's promise to the Person" or "resolve 'by Friday' to a date": do they live in the core, or in a replaceable plugin?
- **Why it matters:** 31 F13: an owner who wants a different business calendar for "Friday" would have to change core. Concept §4 and vocabulary §1 say domain behaviour is a plugin; ADR 0008 rejected an in-core mapper as unswappable. A plugin path needs a claims-in protocol that does not exist (21 A2).
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 4a | Fixed core registry of stratified, acyclic, versioned rules (interim). | Concept tension for domain rules; each rule is core code. | Possibly concept note (31 F13); plugins §. | 21 A2; 21 §4c rules `rastro.lift-to-person/1`, `resolve-due/1` | 21 |
| 4b | Core does only generic validation and maintenance; derivation is a validated plugin whose premises the core checks. | New protocol message and validation; later than slice 3–4. | ADR 0008 (new role/message); plugins. | 31 F13 fix | 31 |
| 4c | Owner-configurable bounded rules (the glossary "Rule"). | Rule language to define; bounded recursion. | Glossary "Rule" use; levels. | 31 F13 option | none |
| 4d | No inference now; the first slices need none (observed only). | Defers lift-to-Person and date resolution to slice 5. | None. | 20 §4c is "outside the paused observed-only contract"; 21 slices 4–7 | none (implicit in both: observed-only scope) |

- **Depends on:** D3. **Unlocks:** D14 (does the schema carry an inference rule registry), D15.

---

### D5. Temporal history: equal timestamps and occurrence shape
- **Question:** When several observations share one capture time (capture times are clamped, never decreasing, not unique), how are occurrences identified and ordered?
- **Why it matters:** 31 F7: A says O, then C, B says O at the same `captured_at` t: the O-closed `[t,t)` and the O-reopened `[t,∞)` collide on the proposed key `(lens, claim_id, start)` (reproduced in SQLite). 30 F5.2: `known_interval` allows a zero-length row while `valid_interval` forbids it; a union of presence intervals erases the recurrence the occurrence rows exist for; D15's "newest interval start" tie depends on claim id.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 5a | Separate occurrence ids; transitions ordered by a stable event key (`captured_at`, evidence id; explicit order for intents); recurrence kept even at equal t. | Occurrence minting inputs to specify (30 F5.3); more rows. | ADR 0009 §Replay order, §Tables. | 31 F7 fix; 20 A3 + §3 (`known_interval.iri`) | 20, 31 |
| 5b | Coalesce timestamp-only: drop zero-length occurrences, record a transition only, state the coalescing rule. | Intermediate states lost at equal t. | ADR 0009 §Tables. | 30 F5.2 fix; 31 "Questions" (accept loss) | 30 |
| 5c | Keep 21's key `(lens, claim, start)` and accept the loss silently. | Duplicate PK on equal-t recurrence. | none | 21 §3 | 21 (as drafted; both reviews flag it) |

- **Granularity (also decide):** occurrence rows per justification with unioned claim presence (20 A3) vs per-lens claim interval with `opened_by`/`closed_by` (21 A3); both reach the same open→closed→open results in their worked examples (20 §4a; 21 §4a).
- **Depends on:** D3. **Unlocks:** D8, D9, D10.

### D5b. Temporal history: "as of" and the served-answer audit
- **Question:** For "as of t" (slice 7), is it enough to re-derive today's interpretation from evidence captured by t, or must Rastro reproduce an answer it served earlier?
- **Why it matters:** An owner who asks "why did you say X on Tuesday?" (concept §7 "every act recorded") gets today's rules applied to Tuesday's evidence unless receipts exist; deleting retired extractor outputs (D8) makes the narrower promise the only one possible (31 F15, "Questions").
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 5b-a | Capture-cutoff reconstruction under today's pinned rules; a cheap publication log of extractor/vocabulary versions per generation. | Not the exact served answer. | None (knowledge §25 already). | 20 Q6a; 21 A3 + Q9a; 11 "Two different 'as of'" | 20, 21 |
| 5b-b | Also keep publication or answer receipts for exact served history. | New precious audit data; policy and retention. | Knowledge §12 (more inputs). | 20 Q6b; 31 "Questions" | none |
| 5b-c | Contextual claim ids that carry temporal qualifiers. | Revises plan D5; breaks content ids/renderer targets (21). | Plan D5; ADR 0009 §Ids. | 20 Q6c; 21 A3 (rejected) | none |

- **Depends on:** D1, D5. **Unlocks:** D8, D14.

---

### D6. Identity: merge representation, grouping and the word "Merge"
- **Question:** After two entities are merged, do we keep both claims and group them when reading, or re-key into one claim? What does a renderer target? What does "Merge" mean in the glossary?
- **Why it matters:** 31 F9: P1 and P2 both have description X and merge into one card; renderer confirms only P1; P2 changes concurrently, no conflict on P1 though the card changed; undoing the merge can expose an unconfirmed duplicate that looked confirmed. 30 F10: the "minimum member id" representative changes when a member joins, re-keying anything stored on it. Knowledge §5 says two sources saying the same thing are *one claim with two supports*.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 6a | **Overlay with grouped, immutable claim ids.** Grouping is part of the normal result: all member claim ids and a group fingerprint; a confirmation targets the original claims atomically; fingerprint is never a claim id. | Group status/revision/limit rules undefined (30 F10.2); changes D15 ordering/limit by default. | Knowledge §5 clarified (one claim per exact statement; equivalents grouped); renderers §5 (target sets); plan D15. | 20 A4 + Q2a; 31 F9 fix | 20, 31 |
| 6b | **Overlay, grouping opt-in** (`about(expand: identity)`), D15 unchanged by default. | Duplicates visible by default; no result schema for member ids/aggregate revision/targets yet (31 F9). | Surfaces (additive parameter); knowledge §5 unresolved. | 21 A4 + A10 + Q3; 30 F10 fix | 21, 30 |
| 6c | **Re-key on merge, with redirects** (option D). | Claim ids change on merge and unmerge; renderer targets need redirects. | ADR 0009 §Ids; renderers §5. | 13 §3, §9; 20 Q2b; 21 A4 (rejected) | none |
| 6d | Defer Person and merge execution. | Slice 4 stalls; identity stays "exact identifier only". | None. | 20 Q2c | none |

- **Also decide:** (i) the glossary word: broaden "Merge" to "…are one, or belong to one Person" / keep Merge = Person↔Person (or same-kind) and call account linking "link" / Person = cluster (13 P3), (21 Q3a/b/c, 20 A4: literal account→Person merging contradicts vocabulary §12); (ii) the display representative: member list only (30 F10 fix) vs min visible member (20, 21, shared); (iii) undo records a cannot-link always vs owner chooses (21 Q12; in D2).
- **Depends on:** D2, D3, D5. **Unlocks:** D7, D9, D13.

### D7. Identity: Person minting and email durability
- **Question:** Is a Person's id derived from a vouched email, from a durable contact or owner intent, or from episodes of ownership?
- **Why it matters:** 31 F10: `ana@example.com` belongs to Ana in January and Bea in June; both accounts reach the same email-minted Person, so commitments aggregate across two humans; rejecting one link cannot give them different Persons with the same canonical key (13 §5 names reassigned work emails).
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 7a | Mint a Person from an exact vouched email (`person:email/<normalised>`), or an intent, or an address-book record; two accounts with the same email share a Person with no merge; blocklist and per-type limits. | Email reuse and provider normalisation (Gmail dots) baked into identity; a changed email makes two Persons joined later. | Knowledge §24 extended; vocabulary §1. | 21 A4 + §4b; 13 §6 | 21 |
| 7b | Durable Person ids (owner intent, address-book resource), with time-bounded email/account **links** as claims; an equal email proposes or justifies a reversible link. | A link per account; fewer automatic joins. | Knowledge §24 not extended; vocabulary §1. | 31 F10 fix; 20 A4 ("Person may be minted from a durable Owner intent ID"; source-vouched exact keys alone can automate) | 20 (partly), 31 |
| 7c | Ownership episodes: Person ids include an episode, plus an owner split operation. | Needs a split mechanism. | Vocabulary §1. | 31 F10 option | none |

- **Depends on:** D6. **Unlocks:** D13, D14.

---

### D8. Several extractors on one source, and retiring old versions
- **Question:** One extractor per source (today), or a list? When an extractor version is upgraded, what happens to the old version's justifications?
- **Why it matters:** 00 §5 A8: a version bump re-reads the pair; main and T4 never retire old-version supports, so rebuild and incremental diverge. 31 F15: with version-free revision keys and a multiset sum, identical v2 output arriving while v1 remains doubles the sum (h→2h) and back, two revision changes with no visible change.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 8a | Explicit bindings list per source now; replacement slot declared per rule; retire the old version **after a complete successful replay** (failed upgrades keep last good output). | More checkpoints, replay barriers; slot declarations must tell "same rule upgraded" from "alternative rule". | ADR 0008 §Binding; plugins §15. | 20 A8 + Q9a | 20 (design); 21 (list, delete on catch-up) |
| 8b | Define slots now; keep one binding until a second extractor exists. | Slot contract exists with no user yet. | None now. | 20 Q9b | 20 (for present scope) |
| 8c | Delete old-version justifications once the new version catches up on a pair (incremental = rebuild). | Old history gone; status during the transition is the union; claims only the old version supported disappear unless an intent sustains them. | Plugins §15; ADR 0008; limits D5b. | 21 A8 + Q13a | 21 |
| 8d | Keep old-version supports as history (current behaviour). | Rebuild ≠ incremental; obsolete interpretations stay current. | None. | 20 Q9c; 21 Q13b; 00 §5 | none |

- **Also decide:** several extractors on the same evidence = interpretations, not independent corroboration (20 A8) vs "one claim with two justifications" counted as support (21 A8, knowledge §5). Authority across extractors: field group excludes the rule (21 A8; T4 already) with an owner override as a Rule.
- **Depends on:** D3, D5. **Unlocks:** D9, D10.

---

### D9. Scope strategy: how scoped reads stay exact
- **Question:** Do we recompute scoped state on each read, materialize it per launched scope set ("lens", a working name, not a glossary term), or start one way and move on a measured trigger?
- **Why it matters:** Scope semantics are non-monotone: a hidden newer observation must not hide a visible claim (knowledge §26, plan S28). 30 F2: read-time sort key is computed so no index stops at `limit+1`; on a 20k-claim hub the same query took 103 s without `ANALYZE`, 35 ms after; and the correct scoped sweep (not the cheap join) is unspecified in 20's DDL. 31 F5: lenses cost up to 2^n−1 scope sets; ten scopes with all sets containing `work` mean 512 copies; GC of a long-lived MCP session's lens falls back to a weaker capped answer. 31 F4: hidden base state (an undo from another scope, a work-only type) can change a visible answer.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 9a | **Read-time recompute** from visible justifications, always; stored full-scope state is only an acceleration hint. | O(degree) per call; planner-fragile (needs `ANALYZE` at activation, which a read-only open cannot do); sweep input and per-call work unspecified. | ADR 0009 §Scope (mechanism). | 20 A6/A10, §3 notes, §6; 30 F2 | 20 |
| 9b | **Lenses:** materialize state (status, revision, intervals, links, adjacency, types, aliases, clusters, degrees) per launched scope set; reads are O(limit). | Storage ~(1 + distinct scopes per claim)× by 21's estimate, but really Σ over registered sets (31 F5); invalidation across merges; registration at launch; fallback when a lens is missing. | New ADR (lenses); amends ADR 0009 §Scope/§Tables; ADR 0002 read path (registration is a write at launch). | 21 A6, §1, §3, R1/R2; 31 F5, F6 | 21 |
| 9c | **Hybrid:** read-time now, with an adjacency-style index and the scoped sweep input in the DDL; read-time is also the oracle; lenses only when a measured gate fails. Or: cache only common lenses, exact scoped computation elsewhere, honest work-budget failure. | Two code paths; the gate and its benchmark must exist. | As 9a, ADR later. | 30 Q3c, F2 fix; 31 F5 alternative | 30, 31 |
| 9d | Materialize only the full scope and accept leaks. | Contradicts D18/S28. | Knowledge §14 amended. | 21 Q1c | none |

- **Required under any option (reviews):** a scope oracle over random scope *sets* with hidden-input add/remove invariance, including artifacts, intents and rules (31 F6; 21 R2 tests singletons only); same-scope or per-lens undo (31 F4); type grounding as a visible derivation dependency, not hidden base state (31 F4).
- **Depends on:** D2, D5, D6, D8. **Unlocks:** D13, D14.

---

### D10. Revision hash inputs
- **Question:** What is in a claim's revision: only the visible state, or also the versions of the rules and vocabulary that produced it?
- **Why it matters:** Renderers target claim id + revision (renderers §5); ADR 0009: "a revision change means the visible state changed". 30 F4: a vocabulary patch (one `definition` typo) or an extractor patch that yields identical supports changes every claim's revision; an owner's pending confirm tick then fails as stale. 31 F15: multiset sums double-count during upgrades. T4 changed the encoding under the v1 tag, which main's own comment forbids (`claims.rs` v1 tag; 21 §5).
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 10a | Visible state only: status, visible support spans/evidence, links, validity. Versions stay on the justification (`why`). | Cannot tell "same state, new rules" from the revision. | ADR 0009 §Ids and revision (v2 tag). | 30 F4 fix, Q6a | 30 |
| 10b | Visible state plus justification ids, premise occurrence refs, policy/term hashes. | Stale-revision Asks on definition edits. | ADR 0009 §Ids; renderers §5 behaviour. | 20 A10, §3 | 20 |
| 10c | As 10a but excludes only `extractor_version`; sum of per-support keys (O(1) update). | Multiplicity during upgrade (31 F15) unless set-deduplicated; collision resistance against a crafted extractor unproven (21 "Unverified"). | ADR 0009; new tag. | 21 A8, §3 step 6, R9; 31 F15 fix | 21 |
| 10d | A canonical sorted support set hashed (O(n)), optimize after profiling. | A claim with 10k supports rehashes (21 A8). | ADR 0009 new tag. | 31 F15 fix; 21 R9 fallback | none |

- **Also decide:** portable evidence key inside the hash (D1 sub-question i); whether a v2 tag is introduced now (D11).
- **Depends on:** D1, D2, D3, D5, D8. **Unlocks:** D11, D14.

---

### D11. Standards profile and id scheme: RDF/PROV/nanopub/JSON-LD now, later, or never; re-version ids once?
- **Question:** Do we commit to a standards-shaped model and id scheme now, publish a profile and serialize later, wait until sharing (slice 16) needs it, or stay internal?
- **Why it matters:** 30 F6: RDF 1.2 Concepts is a CR Snapshot (2026-04-07); Semantics CR Draft; SPARQL/Turtle/SHACL 1.2 WDs; JSON-LD has no triple terms; the interop note is a Group Note Draft (10 "Bottom line"; 20 confirmed 2026-11-05). The roadmap has no consumer: export carries evidence, confirmations, rules, outcomes (sharing §1), not claims; Share is "evidence from a person" (sharing §3). 31 F14: a count-only round-trip test would pass while hypotheses become world assertions; canonicalizing `date` and `datetime` into one claim changes meaning. 30 F7: nothing is stored to protect (owner DB has no projection tables, `00 §4`), but `urn:rastro:` IRIs need a naming authority or Share collides on `code:state`; Person IRI scheme is inconsistent.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 11a | Publish a semantic/export profile now (reified, unasserted claims; PROV mapping; nanopub-style package; RDF 1.1 statement-node fallback); build the serializer when sharing starts. Keep v1 id inputs; legacy-ID↔IRI adapter. | IRI keys and a third object category in the schema; profile tracks drafts; two encodings forever. | ADR 0009; vocabulary; sharing. | 20 A1/A2/A4, Q8a, §5; 31 F14 | 20, 31 |
| 11b | `urn:rastro:` ids only; re-version once (new tags, rename `type`/`alias` to `rastro:` namespace); JSON-LD 1.1 statement-node serializer at the edge when sharing starts. | Ids of the T5 meta-claims change (free now). Other library consumers? (31 "Boundaries"). | ADR 0009 §Ids; ADR 0008 (meta-predicates). | 21 §1/§5; 30 F6, F7, Q4b, Q5b | 21, 30 |
| 11c | No standards commitment and no IRIs until sharing; relational model only. | Interop later requires mapping then. | None. | 30 Q4c | none |
| 11d | Internal JSON only, never a standard representation. | Forgoes an independently inspectable form. | None. | 20 Q8c | none |
| 11e | Nanopub-style serialization shipped immediately. | Scope now. | Sharing. | 20 Q8b | none |

- **Also decide:** who owns the IRI/namespace authority if IRIs stay (30 F7.2); datatype identity: keep lexical identity (`"01"` ≠ `"1"`, 20 A5) vs canonical lexical forms per datatype (21 A5), noting `date` ≠ `datetime` (31 F14).
- **Depends on:** D1–D3, D10. **Unlocks:** D12, D14.

---

### D12. Vocabulary model and validation (SHACL: now, later, never)
- **Question:** How does the core check claims against vocabularies: a compiled SHACL-style profile, fixed Rust checks, a full SHACL engine?
- **Why it matters:** The valuable part is bundle-atomic validation (type claims establish endpoint types regardless of message order, replacing T5's "type claims first" ordering; 30 F9). 20 itself lists ~15 invariants that FK/SHACL cannot replace, so a Rust validator is needed anyway (30 F9). A restricted profile can drift from real SHACL unnoticed because no engine runs in CI (30 F9). Domain inference by RDFS domain/range is not validation (20 A5).
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 12a | Fixed Rust checks plus conformance fixtures; SKOS/SHACL only as export when a consumer exists ("later"). | Every new vocabulary feature is core code. | Vocabulary (core meta-model, datatype registry); ADR 0008. | 21 A5; 30 F9 | 21, 30 |
| 12b | Bounded SHACL-style profile compiled from installed vocabularies, SHACL 1.0-compatible export, differential fixtures ("now", small). | Profile compiler; a SHACL engine needed in CI for the fixtures (not identified); `domain_json` is not relational. | ADR 0008 §Core validation; vocabulary. | 20 A5 + Q7a | 20 |
| 12c | Full third-party SHACL implementation ("now", large). | Larger core and query attack surface. | ADR 0008. | 20 Q7b | none |
| 12d | Bespoke schema with no standard mapping ("never"). | No interop shape. | Vocabulary. | 20 Q7c | none |

- **Required under any option:** bundle-atomic type grounding (21 "type grounding"/anchor; 20 A5); `deny_unknown_fields`, required `cardinality` (C11); optional `supertype` and cross-namespace dependencies at pinned versions (20 A5, 21 A5); overlap mappings are confirmed claims (vocabulary §7); re-validate vs re-extract on a definition change (D1 iv).
- **Depends on:** D11. **Unlocks:** D14, D15.

---

### D13. Agent query surface and expansion
- **Question:** Beyond `search`, `coverage`, `about` (one hop) and `why`, may agents expand from search hits or across identity, and with what bounds?
- **Why it matters:** 14: a bot with 10k authored PRs is noise for some questions and the answer for others. 31 F8: `ORDER BY` + `LIMIT` does not bound work (one returned row cost 80,056 VM instructions on 21's adjacency query) and grouping after a page fetch can underfill the page. 30 F12: surfaces §23 says a bare name is an error; 20 A10 says bare names are search candidates.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 13a | Fixed operations only (`about` one hop, D15 order, 20/50), with vocabulary references in answers. | Several round trips; no "count all X". | Surfaces: bare-name rule (30 F12). | 20 Q5b; 21 Q10b | none |
| 13b | Fixed operations now; bounded expansion added after it beats FTS in an equal-token evaluation on 40–60 owner questions. | Evaluation first; budgets (5 seeds, depth ≤2, 25 neighbours, 2,000 examined, 50 out, 64 KiB; "need measurement"). | Surfaces (new operation, catalog decision). | 20 A6/A10 + Q5a; 14 "Evaluation" | 20 |
| 13c | Additive bounded options now: `about(expand: identity)`, `search(claims: true)`, `why(premises)`; BFS with per-lens hub cut-off and `truncated_reasons`. | More surface to test; hub thresholds fixed vs per vocabulary (21 Q10). Needs index with `live` first (31 F8). | Surfaces (additive parameters). | 21 A6/A10 + Q10a, R4 | 21 |
| 13d | Restricted SPARQL on MCP. | Unbounded work, no row-level scope joins. | Surfaces; ADR 0007. | 20 Q5c; 14 | none |

- **Also decide:** owner-facing ad-hoc queries: `edge` SQL view in CLI + RDF export / a `rastro query` surface / none (21 Q14).
- **Depends on:** D6, D9, D11. **Unlocks:** D14, D15 (T6, T8).

---

### D14. Schema, migration staging and publication mechanism
- **Question:** How much of the model goes into the next migration, and how is a rebuilt projection published?
- **Why it matters:** 30 F3: for an observed-only slice, `20`'s 19-table DDL needs 12; `merge`, `owner_event`, `confirmation`, `artifact`, `artifact_input`, `premise`, `valid_interval` are for later and not separated. Migrations are never edited (ADR 0002, `00 §6` inv. 22), so what ships stays. 30 F11 / 21 A7: generation keys prefix every index and query; TEMP copy publication is O(projection) and doubles the WAL. Owner data today: 1,520 evidence rows, 714 locations (`00 §4`).
- **Options (staging):**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 14a | One migration for the observed-only subset now; separate later migrations for intents, artifacts, merge, valid time. | Later migrations must fit what ships. | ADR 0009 §Tables. | 30 F3 fix; 20 §5 migration steps 1, 4 | 30 |
| 14b | Full model schema now (20: 19 tables; 21: ~25 incl. lens tables) after the owner approves the complete model. | Premature tables; larger review. | ADR 0009 superseded. | 20 Q10b; 21 §3 | none stated |
| 14c | Replace main's T3 projection tables with one new migration `knowledge_v2` carrying groups B and C; T4's unmerged migration never applied. | Drops disposable tables (owner DB has them unapplied). | ADR 0009 §Tables; ADR 0002 migration rule. | 21 §3, §5 | 21 |

- **Options (publication mechanism):**

| Opt | Means | Costs | Evidence | Leans |
|---|---|---|---|---|
| P1 | T4's TEMP shadow tables swapped by DELETE + INSERT in one transaction; explicit TEMP DDL instead of stripping `REFERENCES`. Trigger to leave: publication > 60 s or WAL > 2× DB (21 R10). | O(projection) copy at publication. | 21 A7, R10; 30 F11 | 21, 30 |
| P2 | Generation-keyed tables with an active pointer and manifest (input high-water, versions). | Prefix on every key and query; retired generations' collection rule under a read-only reader path (30 F11). | 20 A7, §3 | 20 |

- **Also decide:** `ANALYZE` at activation (30 F2.2); insert-only triggers on each precious table (30 F8.3); `valid_time` stored as a CHECK constant vs absent (30 F12, 00 §3); hard-coded `Source::github` in rebuild goes (all).
- **Depends on:** D1–D13. **Unlocks:** D15.

---

### D15. Fate of T4/T5 and the shape of the next slice
- **Question:** Rework T4/T5 onto the chosen schema, replace the paused plan, or something else; and what does the next slice contain?
- **Why it matters:** `00 §3`: the merge is textually clean but does not compile (`claims.rs:288`); T4 has no review, T5's converge FAILS (7 untagged tests); T4's lifecycle refresh replays all supports on every batch (`lifecycle.rs:45–63`). 30 F3 / 31: "resume" understates the work, since little code but fixtures, ids and extractor rules survive.
- **Options:**

| Opt | Means | Costs | Amends | Evidence | Leans |
|---|---|---|---|---|---|
| 15a | Rework both onto one new migration (T4′, T5′); merge neither. Keep: T4 lifecycle semantics, rebuild mechanism, ClaimView fields, test fixtures; T5 extractor crate, URL rules, vocabulary format, alias-quote check. Rework: refresh, `Source::github`, in-place type/alias, location context documented. | About two reworked tickets before T6–T8 (21 estimate). | Plan (tickets T4′/T5′). | 21 A11 + §5; 20 A11 + §5; 30 Q9a; 31 agreements | 20, 21, 30, 31 |
| 15b | Replace the paused plan with the complete approved model. | A full re-plan; delays observed-only value. | Plan replaced. | 20 Q10b | 20 (as the alternative) |
| 15c | Merge both as they are, then refactor. | Does not compile; T4's migration would become an applied file never editable. | n/a | 20 Q10c; 21 A11; 00 §3 | none |
| 15d | Drop both and restart. | Loses ~2.5k lines of tested semantics (21). | n/a | 21 A11 | none |

- **Which decisions gate which tickets (plan table):**

| Ticket | Gated by |
|---|---|
| T4′ lifecycle + rebuild | D2 (precedence), D3, D5, D8, D9, D10, D14 |
| T5′ GitHub extractor | C9, C11, C13, D6/D7 (meta-predicate names, `held_by`), D11 (ids), D12 |
| T6 `about`/`why` | D6 (grouping), D9, D13 |
| T7 failure paths | D1 (artifacts out of scope if observed-only), D14 |
| T8 MCP + scopes | D9 (scope oracle), D13 |
| T9 owner sign-off | T5′, T7, T8 |

- **Depends on:** D1–D14.

## 4. Open facts to check before deciding
Nobody verified these; each could change a decision.

| Fact | Changes | Cheapest check |
|---|---|---|
| Merged T4+T5 tree: any compile or semantic break beyond `claims.rs:288`; and that T5's `cardinality = "one"` parses as `One` once merged (`00` Unverified; 21 header; 31 agreements) | D15 (how big the rework is), C11 | `git worktree add` a scratch tree at merge `6358157` and run `cargo check`/`cargo test` |
| Read-time scoped cost with the **real** scoped sweep, with and without `ANALYZE` (30 F2: 103 s vs 35 ms on a toy join; the sweep itself not measured) | D9 | Re-run 30's 20k-hub script (its scratch dir is named in 30 header), then add the field-group sweep |
| Lens storage under overlapping scope sets (31 F5; 21 R1 benchmarks 3 lenses) and how many scopes the owner really uses (`00 §4` shows `work`, `personal`) | D9 | `grep scope ~/.config/rastro/config.toml`; synthetic with n=3,5,10 scopes |
| Adjacency `LIMIT` really stops early: 31 F8 measured 80,056 VM instructions for one row | D13, D9 | Re-run 31's query with `EXPLAIN` and an index including `live` |
| SQLite version bundled with `rusqlite` vs those used in the probes (3.4x in 21, 3.54.0 in 30, 3.50.4 in 31) | Any plan-dependent claim | `cargo tree -i libsqlite3-sys` |
| Other installs with the projection migration applied (`00 §4` is one owner's snapshot; main already ships `202611061000`) | D11 (re-version ok?), D14c | `git tag --contains` on that migration's commit; ask whether anyone else runs main |
| Scale: owner DB has 1,520 evidence rows, 714 locations, no supersession history yet (`00 §4`); all 100k–10M claim targets are proposals, not measurements (12, 20 §6, 21 §6) | D9, D14, D13 (how much perf engineering is premature) | Decide a target size, or measure after T5′ on real data |
| `revision` hashes the local `evidence_id` (verified in `crates/rastro-core/src/claims.rs`), so cross-machine replay yields different revisions (30 F8.1) | D1, D10 | Done; only the portable-key choice remains |
| RDF 1.2 / SPARQL 1.2 / SHACL 1.2 / JSON-LD triple-term status (10 "Unverified"; 20 re-checked Concepts/PROV/SHACL 1.0 on 2026-11-05) | D11 only if 11a | Re-fetch the W3C status pages |
| Oxigraph cross-process read-only open is undefined (20 A7 checked docs.rs 0.5.13; 30 did not re-verify; 12 Unverified) | Only if the owner considers an engine (C1) | Read the `open_read_only` doc page |
| Slack `users:read.email` as a vouched identifier (21 Unverified); Gmail/Outlook email normalisation (13 Unverified) | D7a | Slack API doc page |
| GitHub repository/issue numeric id after a transfer (13 Unverified) | Only to revisit knowledge §30 | GitHub REST docs |
| Hypothesis precision of the LLM path (15 Unverified; 20 R9 proposes ≥95%) | Slice 5, D1 sub (v) | Owner-labelled fixtures; not needed for observed-only |
| Multiset-hash collision against a crafted extractor (21 Unverified) | D10c | Short analysis, or choose D10d |
| Datalog crate for the roll-up (10 "Leaning"; 21 R3) | Optional; D5/D9 hand-rolled temporal code | A prototype of ADR 0009 roll-up in Nemo/ascent |

Not in the sequence, no design asked for a decision: repository rename/transfer (knowledge §30, 13 §7 options), "forget a Person" across a cluster (13 §10 q7), cross-machine import of precious inputs beyond the portable key (D1), a SQL/PGQ or DuckPGQ sidecar (12 option E; 21 R5 trigger).

## 5. What gets written after the decisions
Per AGENTS.md: product decisions in `docs/decisions/<scope>.md` dated and out of `open-questions.md`; architectural ones as ADRs (numbers allocated only after approval; 21 proposes 0010–0013, 20 proposes no numbers); glossary changes deliberate and reviewed; the concept untouched unless noted.

| Decision / option | ADR | Decision-file entry | Glossary |
|---|---|---|---|
| **D1** 1a | New ADR: extraction artifacts and replay (recorded vs re-extracted), retention, forget, export (20 §5; 21 ADR-0013) | knowledge §12 amendment; sharing §1 (Export) | **New terms to flag:** "extraction artifact" (proposed; not yet a glossary term) |
| D1 1b / 1c | ADR on what replay guarantees weaken | knowledge §12 note | none |
| **D2** any | ADR on justifications and owner intents (21 ADR-0010) or in the identity/justification ADR | knowledge entries: retraction binding, precedence, scope of intent, carry-through delivery; possibly a concept §1 note if 2a with scoped retract (30 F12) | Check `owner_event` kinds against Intent list (`confirm, accept, edit, reject, answer, undo, stop, promote, missed`): 20's `retract`, `undo_confirmation`, `block_merge`, `reverse_rule` are not glossary terms (30 F12) |
| **D3** 3a | Amend ADR 0009 §Tables; ADR 0008 §Core validation | knowledge §5/§6/§14 notes on forgetting of AND bundles | none |
| **D4** 4a | Amend ADR 0008 (core rule registry) | vocabulary/plugins note; **concept note** (31 F13) | none |
| D4 4b | ADR: derivation plugin role and protocol | plugins entry | "Derivation" would be a new plugin kind: **proposed, flag** |
| **D5, D5b** | Amend ADR 0009 (occurrences, equal-time ordering, publication log); an audit receipt choice adds to knowledge §12 | knowledge §25/§26 sharpened (`superseded_by` = current transition) | "occurrence" is an implementation term (proposed, not for glossary) |
| **D6** 6a/6b | ADR: reversible Person merges, grouping, intent targeting (both designs) | knowledge §5 clarification (one claim per exact statement; equivalents grouped); renderers §5 (target sets); surfaces (grouping/expand) | **Merge** wording (broaden / keep + "link"); possible new term "link" (21 Q3b); "cluster", "lens", "slot", "field group", "anchor" stay implementation names (21 header) |
| D6 6c | ADR on re-key + redirects; supersedes ADR 0009 §Ids | renderers §5 | Merge unchanged |
| **D7** | In the identity ADR | knowledge §24 extension (7a) or not (7b); vocabulary §1/§12 (Person link predicate, core meta-model) | Alias/Person entries may need a line (7b/7c) |
| **D8** 8a/8c | Amend ADR 0008 §Binding; plugins §15 | plugins note (binding list, retirement rule) | none |
| **D9** 9a | Amend ADR 0009 §Scope | knowledge §14/§26 (how) | none |
| D9 9b | New ADR: scope lenses (21 ADR-0011), amends ADR 0009 and ADR 0002 read path (registration at launch) | surfaces/`levels` none | "lens" would be a new implementation term (21: not glossary) |
| **D10** | Amend ADR 0009 §Ids and revision (new tag) | renderers §5 note if staleness behaviour changes | none |
| **D11** 11a | New ADR or decision on a portable semantic profile | sharing entry; vocabulary (IRI authority) | none |
| D11 11b | Amend ADR 0009 §Ids (re-version once); ADR 0008 (`rastro:` meta-predicates) | vocabulary §1 | none |
| D11 11c/11d | Decision entry "no standards commitment until slice 16" | sharing | none |
| **D12** | Amend ADR 0008 §Core validation | vocabulary: core meta-model predicates, `cardinality` required, datatype registry, supertype, dependency rule (§§1, 3, 4, 13) | none |
| **D13** 13b/13c | Amend ADR 0007 (additive operation parameters) | surfaces: `expand`, `claims`, `premises`; §23 bare-name rule | none |
| **D14** | Amend/supersede ADR 0009 §Tables, §Writing (publication) | none | none |
| **D15** | none (plan update) | open-questions: move decided items out; roadmap slice text | none |

Also: whichever option is chosen, `docs/open-questions.md` entries get moved to decisions as they are decided, `docs/plans/2026-11-04-claims-and-about.plan.md` "Pause" gets resolved or the plan replaced, and the 00 §3 and 20 §5 cardinality error is noted where `00` is cited.
