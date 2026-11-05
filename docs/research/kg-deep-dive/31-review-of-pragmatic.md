# Cross-review of the pragmatic architecture

2026-11-05 · `review-of-pragmatic` · Semantic architect's side; recommendations only. **The Owner decides.**

## Verdict

Keep SQLite, immutable statement IDs, explicit justifications, scoped projections and recorded stochastic outputs.
These are strong choices; RDF storage would not supply the missing truth-maintenance policy.
The proposed implementation does **not yet meet every concept requirement**: its keys cannot represent existing T5 output, and its intent dependencies are not replayable as specified.
Lenses can be correct, but their storage estimate, invalidation contract and isolation oracle are insufficient.
Stable IDs alone do not solve merged-card targeting; occurrence timestamps alone do not solve equal-time history.
Leaning: retain the relational engine; adopt the semantic design's explicit support bundles, grouping targets and portable semantic profile before approving the schema.

## Evidence and checks

Read the brief, both designs, `00`, `10`–`15`, concept, glossary, all decision files, ADRs 0001–0009 and the paused plan. Design references below mean [21-arch-pragmatic.md](21-arch-pragmatic.md); contrast references mean [20-arch-semantic.md](20-arch-semantic.md). Research citations are local evidence, not independent re-verification of external products.

Read-only revision checks: main/this checkout `4f138e3759b6cd76155af955d6895bb96af2a1cb`; T4 `4ac017f3a72700b6a7d75cc582c926a1b50c4090`; T5 `7a532bbb76e0dcb5d607c99e0901432a7d498475`. `T4:` / `T5:` paths mean `../rastro-ai.worktrees/claims-t4/` / `claims-t5/`. No branch build, real database access, network call or git mutation. SQL probes used Python's SQLite **3.50.4**, `:memory:` only; exact design DDL plus empty `sources`, `runs`, `evidence` parent-table stubs. Foreign-key enforcement was not enabled in these structural/query probes; uniqueness and query execution were exercised. DDL parsed successfully.

## Findings — severity ordered

### F1 · blocking · Justification and slot keys lose existing T5 claims

- **Claim:** §3 `justification.jkey = H(kind, evidence, span, rule, extractor_version | …)` is globally UNIQUE; scalar `slot = H(source, scope, location, field, rule, extractor_version)`. §5 keeps T5's extractor.
- **Wrong:** Neither key identifies the conclusion; the slot also omits subject/predicate. A rule can legitimately emit several conclusions from one span. The slot must distinguish their independent fields, while deduplication must distinguish their claims and producing extractors.
- **Failure:** T5 `repository-from-url` emits `PR in_repository Repo` and `Repo type Repository` from the **same** `/state` support → identical jkeys → second insert fails or is silently discarded. Merely adding claim ID to jkey still leaves both scalar claims in one slot, where §3's close-on-different-claim rule makes them replace each other.
- **Evidence:** `T5:crates/rastro-extractor-github/src/main.rs:160` and `:171–186`; T4's slot already includes subject/predicate in `T4:crates/rastro-storage-sqlite/src/lifecycle.rs:9,24–40,54`. In-memory insertion of two conclusions with the prescribed equal jkey: second raises `UNIQUE constraint failed: justification.jkey`.
- **Fix:** Include claim ID and extractor identity in justification identity. Restore subject/predicate to scalar slots; explicitly define atomic multi-conclusion observations. Preserve the rule-independent *field group* for cross-extractor conflict detection.

### F2 · blocking · Required evidence spans become independent OR proofs

- **Claim:** §2 A2 models a reason as one justification, AND over premises and OR over justifications. §4c's `owes` provenance needs author evidence **and** message evidence. §3 gives a justification exactly one evidence ID/span.
- **Wrong:** There is no child support-span table or equivalent evidence-AND bundle. `artifact_input` records everything a model read; it does not say which inputs each conclusion requires. Duplicating provenance rows turns required inputs into alternative proofs.
- **Failure:** `author(PR,Alice)` requires `/user/id=42` AND `/user/type=User`. Store two provenance justifications; forget the ID evidence → type-only justification still supports authorship. Store just the ID → type evidence and its forgetting dependency disappear from `why`.
- **Evidence:** Existing T5 already supplies both supports: `T5:crates/rastro-extractor-github/src/main.rs:195–217`; alias additionally needs ID plus login (`:220–228`). Knowledge §6/§14, ADR 0008 §Core validation; `11-time-provenance.md §Justifications and retraction`. Exact §3 DDL has no support-span child table.
- **Fix:** One identified derivation with an AND set of required evidence spans plus an AND set of claim premises; OR only between complete derivations. Adopt semantic §3 `support_span` shape and explicitly define its liveness/forgetting semantics.

### F3 · blocking · Anchor confirmation creates a cycle; its dependencies are disposable

- **Claim:** §4c confirms five targets, including the anchor; “Each confirmation justification gets premise = anchor.” §2 A2 bans cyclic inference. §3 keeps `intent_target` precious but `justification_premise` disposable.
- **Wrong:** The anchor's confirmation depends on itself. Banning cycles only in *inference rules* misses confirmation/provenance edges. The precious intent records neither candidate membership nor anchor/premise links, so rebuilding its promised dependencies is under-specified, especially after extractor removal.
- **Failure:** Confirm anchor H and slot D, then retire H's original extractor. H's remaining justification requires H → either replay cannot establish it, or a prior IN label makes it self-supporting. Independently, rebuild from the surviving intent's tuples cannot determine that D must retract when H is rejected → D survives, contrary to §4c.
- **Evidence:** Design §3 `intent_target`, `justification_premise`, rebuild steps; §4c steps 2–4. Knowledge §10/§12; `11-time-provenance.md §Justifications and retraction` cycle warning. Semantic §2 A9 instead distinguishes exact confirmed bundles and undo-yes from explicit retraction.
- **Fix:** Exempt the anchor from its own premise; reject cycles over the **whole** dependency graph. Persist reviewed bundle identity and dependency/occurrence relationships with the intent. Specify undo-of-undo, deterministic equal-time intent ordering, and precedence at each historical cutoff, not only current status.

### F4 · major · Scope-dependent inputs mutate shared base state; type grounding can depend on hidden data

- **Claim:** §3 base state is scope-independent; undo closes confirmation `end_at` in the base (§4b). Q5 defaults intent scope to its current context. A5 accepts endpoint types grounded “in this run or the base.” Incremental updates only lenses covering the batch scope.
- **Wrong:** No rule requires an undo's scope to match its target intent. A hidden undo can therefore close visible support globally. Likewise, accepting/rejecting a personal submission based on a work-only type claim makes personal knowledge depend on hidden evidence before lens computation even begins.
- **Failure:** Personal confirmation C; owner undoes C from work context → work-scoped intent closes C's base justification. A later personal recompute loses C despite no visible undo; an untouched personal lens may initially keep it. Or submit a personal relation whose only endpoint type is work-only → accepted only in the full DB, rejected when rebuilt from personal inputs.
- **Evidence:** Design §3 write paths/steps 1, 4 and §4b; knowledge §14/§26; surfaces §26; ADR 0009 §Scope; plan S28. Semantic §2 A5 makes type dependencies premises.
- **Fix:** Enforce same-scope undo or compute its effect per lens. Type grounding must be a visible, recorded derivation dependency; do not let hidden base state determine admission. Test both full→scoped and independently scoped rebuilds.

### F5 · major · Lens cost and lifecycle are not bounded by distinct scopes per claim

- **Claim:** §1 estimates `(1 + distinct scopes per claim) × claims`; §3 registers every launched scope set and recomputes covering lenses. §1 garbage-collects unused lenses; §4d falls back to a bounded neighborhood if absent.
- **Wrong:** Cost depends on **registered sets containing each visible scope**, not the number of scopes on a claim. With n scopes there are up to `2^n−1` nonempty launch sets. Component-wide identity changes also exceed the stated touched claims plus their endpoints. Registration needs a complete snapshot; GC needs reader/session lifetime rules.
- **Failure:** All N claims have only `work` support; launch all sets containing work among ten scopes → 512 copies of N claim states, intervals, adjacency and entity state, not about 2N. GC a long-lived MCP session's lens → next read takes a capped fallback that cannot promise the same complete top-k/status roll-up. A–B–C merge then remove B–C → updating only B/C leaves A's cluster stale.
- **Evidence:** Design §3 `lens`, write paths, `recompute` step 7; §6 R1 benchmarks only three lenses. `13-identity.md §3–4` requires component recomputation. ADR 0002 read-only path; ADR 0009 atomic answers.
- **Fix:** Cost formula `Σ_L state(visible inputs in L)` plus all indexes/history; benchmark overlapping launch sets and fresh-lens latency. Specify active-session leases/pinning, atomic registration and full affected-component invalidation. Alternative: cache only common lenses; exact scoped roll-up elsewhere, with honest work-budget failure rather than silently weaker results.

### F6 · major · The scope oracle is useful but incomplete

- **Claim:** §6 R2 tests each singleton `{S}` against an unscoped DB rebuilt from S's evidence and intents; any difference except FTS rank is a leak/bug. R3 also compares with T4 naive replay.
- **Wrong:** Singleton tests miss multi-scope AND derivations and union-only identity bridges. The replay inputs omit newly precious artifacts and applicable owner rules. Two equally wrong implementations can agree. T4 does not implement these new semantics and cannot be the unchanged oracle for them.
- **Failure:** Derivation D requires personal A AND health B. Bug admits D into `{personal,work}` through an incorrect union rule but keeps all singleton answers correct → R2 passes. Alternatively, a stochastic claim disappears in the comparison DB because its artifact was never replayed → false failure, not proof of a leak.
- **Evidence:** Design §2 A2/A9, §6 R2/R3; `15-llm-ontology.md §9`; T4 `lifecycle.rs:45–201` handles provenance/current support only. Semantic §6 also proposes hidden-input metamorphic testing.
- **Fix:** Random scope **sets**, hidden-input addition/removal invariance, and independent expected-answer fixtures. Include applicable artifacts, scoped intents, rules and vocabulary versions; preserve/map evidence IDs used in revisions. Extend naive replay separately; do not call existing T4 an oracle for capabilities it lacks.

### F7 · major · Equal-time A→B→A cannot fit the occurrence key or acyclicity proof

- **Claim:** §2 A3/§3 use `PRIMARY KEY(lens,claim_id,start)`; links are acyclic because successive occurrence starts strictly increase. §4a discusses tied source clocks, not tied capture times.
- **Wrong:** Capture times are non-decreasing, not unique. Timestamp boundaries alone discard the accepted evidence-ID replay order. If zero-duration occurrences are retained, strict-increase reasoning fails; if coalesced away, intermediate observations/history disappear.
- **Failure:** Same capture timestamp t: source A evidence 1 says O; A evidence 2 says C; source B evidence 3 says O, with successively newer comparable update clocks → replay O→C→O. Both O occurrence starts are t → duplicate PK. Sources can share t; main also clamps backward capture times to the pair maximum.
- **Evidence:** Main `crates/rastro-storage-sqlite/src/lib.rs:303–312`; ADR 0005 §Re-observation, ADR 0009 §Replay order; plan S9. In-memory insertion of O's closed `[t,t)` and reopened `[t,∞)` raises `UNIQUE constraint failed: claim_interval.lens, claim_interval.claim_id, claim_interval.start`.
- **Fix:** Separate occurrence IDs; order transitions by a stable event key including evidence ID and an explicit ordering for intents. Define timestamp-only as-of at ties. Prove acyclicity using transition order, or retain occurrence-level replacement edges as in semantic A3; content-level links are only the current presentation.

### F8 · major · LIMIT does not give the claimed O(limit) reads; grouping can underfill pages

- **Claim:** A6/§4d adjacency query stops at limit+1 “whatever the degree”; per-member top-k plus k-way merge gives exact top-k after grouping.
- **Wrong:** Primary key lacks `live` before time, so rejected/historical rows are scanned and filtered. The nullable `since` OR is not a reliable range bound. Identity grouping happens **after** fetching finite pages; duplicates can consume every slot. Self-loops also have both directions in adjacency.
- **Failure:** Hub has 20,000 newer non-live rows and one old live row; limit 20 → one output requires scanning the hub. Each of two merged subjects has 20 newest `related_to` claims pointing at 20 members of a single merged object cluster; all 40 collapse to one group, with older distinct groups still available → fetching only 20/member cannot establish the requested top 20 groups.
- **Evidence:** Exact DDL/query on SQLite 3.50.4, `ANALYZE`: `SEARCH adjacency USING PRIMARY KEY (lens=? AND entity_id=?)`; one returned row cost **80,056 VM instructions**. No temporary sort is necessary, but that does not prove 21 rows examined. `14-retrieval.md §Index design` separates output and examined work.
- **Fix:** Current-only partial ordered index or `(lens,entity,live,known_since DESC,claim_id)`; separate SQL for since/history cases. Deduplicate claim/direction before limiting. Continue member streams until k distinct groups or an explicit work cap; report incomplete grouping when capped.

### F9 · major · Overlay grouping leaves knowledge §5 and renderer targeting unresolved

- **Claim:** A4 says “one claim, two supports” is satisfied by read-time grouping; §5 says knowledge §5 and renderers §5 are kept.
- **Wrong:** Grouped equivalents remain separate claims with separate statuses/revisions. There is no result schema for member IDs, aggregate revision, conflict handling or intent targets. Identity expansion being optional also means deduplication is not the universal default.
- **Failure:** P1 description X and P2 description X merge; card shows one X. Renderer confirms only P1's ID/revision; P2 changes concurrently → no conflict on P1, although the grouped card changed. Confirming a fabricated group ID instead finds no claim. Undoing the merge can expose an unconfirmed duplicate that looked confirmed on the card.
- **Evidence:** Knowledge §5; renderers §5; `13-identity.md §3,§9` explicitly calls overlay grouping a gap. Semantic A4 acknowledges the knowledge §5 amendment and returns all original targets plus a group fingerprint.
- **Fix:** Adopt that explicit grouped-result/target contract and obtain a knowledge §5 clarification/amendment. Validate the whole reviewed target set atomically. ID stability is necessary; it does not by itself establish card stability.

### F10 · major · Email-derived Person identity bypasses reversible identity decisions

- **Claim:** A4/§4b mint the same Person directly from equal vouched email; “no merge at all.” Blocklists and per-type limits mitigate bad matches.
- **Wrong:** A source can vouch for an account's current email without proving a mailbox belongs to one human forever. Baking email into canonical Person identity makes a mistaken equality occur before the reversible `same_as` layer. No lifetime/disambiguation policy or split operation is specified.
- **Failure:** Work mailbox `ana@example.com` belongs to Ana in January and Bea in June; both accounts map to the same Person → commitments aggregate across different humans. Rejecting one `held_by` disconnects that account but cannot assign the two humans distinct Persons with that same canonical key.
- **Evidence:** Concept §1 “One identity per thing” and “Correctable”; knowledge §4; `13-identity.md §5` explicitly identifies reassigned work emails and shared identifiers. Design A4's later merge handles *changed* emails, not reused ones.
- **Fix:** Prefer durable contact/intent Person IDs and time-supported email/account links; equal emails propose or justify a reversible identity claim. Alternatively define ownership episodes and a durable owner-controlled split/disambiguation mechanism. Blocklisting common mailboxes is insufficient.

### F11 · major · Precious artifacts lack export, forgetting and re-extraction semantics

- **Claim:** §1/§3 artifacts are insert-only, exported precious inputs; §3 IDs hash request identity and UNIQUE `(extractor,extractor_version,request_hash)`; re-extract writes new artifacts. §6 R7 says deleting one changes only its claims.
- **Wrong:** No immutable guards are supplied for artifacts, no active-artifact/interpretation replacement relation, no portable mapping for local run/evidence IDs, and no specified forgetting closure for raw response/submissions/intents. Same request with a fresh stochastic output has the same prescribed identity. Artifact outputs can also support downstream inference, so “only its own claims” is too strong.
- **Failure:** Re-extract unchanged evidence/prompt/settings/version, receive a different response → UNIQUE collision or reuse of the old output. Forget one private message among several inputs → its words remain in `response`/`submissions` or an independently sustaining confirmation and reappear on export/rebuild. Retain both interpretations without retirement → obsolete hypotheses survive beside replacements.
- **Evidence:** Knowledge §2/§12/§14; sharing §1; `15-llm-ontology.md §9`; design §3 `extraction_artifact`, `artifact_input`, `intent_target`; semantic A9/§5 explicitly includes payload purging and portable replay.
- **Fix:** Separate request identity from immutable execution/output artifact identity; define successful-run activation/retirement and failure recovery. Document scoped portable export/import with ID remapping, complete request context and validator versions, immutable guards, and privacy markers erasing every copied payload. Test offline restore into a fresh DB and forgetting through confirmed/inferred claims. Amend **sharing §1** as well as knowledge §12; export is distinct from optional RDF claim sharing.

### F12 · major · Carry-through is promised without a crash-safe execution interface

- **Claim:** A9/§4c emit changed-claim events; execution graph matches `(claim,revision)`, flags Action runs, proposes undo and withdraws pending Proposals.
- **Wrong:** There is no durable delivery/recovery contract tying knowledge commit to these effects, nor a rule to re-evaluate alternative proposal bases. Claim-only premises also leave recurrence binding unclear: a dependency on January's occurrence can revive in June unless intentionally recomputed.
- **Failure:** Commit anchor rejection, crash before event delivery → restart sees checkpointed retraction, pending reminder remains and executes. Or Proposal P has J1=A∧B OR J2=C; A retracts → blanket basis-match withdrawal wrongly discards still-supported P.
- **Evidence:** Knowledge §10; levels §3; plugins §7–§8; `11-time-provenance.md §Justifications and retraction` requires alternatives and occurrence/revision references. Paused plan D20 explicitly defers event delivery, so existing T4 tests cannot verify this promise.
- **Fix:** Specify durable outbox/reconciliation and idempotent consumers, atomic with projection publication or authoritative intent acceptance. Persist the execution basis and its AND/OR/occurrence semantics outside disposable projection state. Re-evaluate Proposals; flag historical Action runs and propose undo, never claim external effects were reversed.

### F13 · major · Domain inference becomes unchangeable core policy without flagging the concept tension

- **Claim:** A2 allows only a fixed core Rust inference registry; §4c uses `rastro.resolve-due/1` and `rastro.lift-to-person/1`. A11 says this still meets every concept requirement.
- **Wrong:** A temporary restriction can be reasonable, but domain interpretation such as relative-date resolution belongs to replaceable interpretation policy. No plugin seam or extension path is specified for these core-only rules. A5 also says every unsupported vocabulary feature requires core code.
- **Failure:** Owner wants a different “by Friday” business-calendar interpretation → must fork/change core; an extractor cannot replace the active core inference policy through the proposed contract.
- **Evidence:** Concept §Who it is for (“Every part an owner would want to change … without forking”), §4; vocabulary §1 standard/plugin boundary; ADR 0008 §Considered options rejects the in-core mapper as unswappable. This tension is not listed among the design's concept amendments.
- **Fix:** Restrict core to generic validation/maintenance. Place domain derivation in a replaceable plugin with validated premises, or explicitly mark an interim limitation and obtain Owner agreement on its boundary. Neither RDF nor unrestricted plugin recursion is required.

### F14 · major · A standard semantic profile is deferred until after meaning-changing choices

- **Claim:** §1 interop/validation stay bespoke; A5 canonicalizes literals before hashing, including the example that a date and midnight datetime should not be two claims. §6 R12 checks RDF round-trip predicate/status counts and may drop RDF export after three months without use.
- **Wrong:** SQLite does not require postponing a portable semantic contract. Counts do not establish preservation of proposition identity, unasserted hypotheses, scoped justifications, occurrences, dependencies or owner confirmation. Date and datetime are different typed meanings; within-datatype canonicalization alone cannot equate them. Existing hash inputs include datatype and lexical value.
- **Failure:** `due_by "2026-11-06"^^date` is translated to midnight UTC datetime → a calendar-day claim acquires an instant/timezone it never asserted. Or export preserves five hypotheses' status counts but asserts their world triples → a generic consumer reads them as accepted facts. Both pass the proposed count test.
- **Evidence:** Main `crates/rastro-core/src/claims.rs:74–100`; ADR 0009 §Ids; `10-models.md §1,§6`; `11-time-provenance.md §Provenance representations`; semantic A1/A2/A5 distinguishes reification, assertion and exact lexical identity.
- **Fix:** Publish a small versioned profile now, serialize later: injective term/ID mapping; typed-literal identity; asserted versus reported statements; exact spans, dependencies, scopes and temporal transitions; interpretation/version provenance. Differential fixtures can use RDF 1.1 statement nodes, SKOS/SHACL-compatible definitions and PROV mappings without a native RDF engine. Any canonicalization migration must preserve legacy targets or explicitly version/remap them. Optional RDF export may be deferred; portable precious-input export remains required by sharing §1.

### F15 · minor · Version-independent revision keys still double-count during upgrades

- **Claim:** A8 says identical supports across an extractor upgrade leave revisions unchanged because `rkey` excludes version; §3 sums rkey hashes as a multiset, while A8 keeps both versions until catch-up.
- **Wrong:** Distinct jkeys for v1/v2 yield the same rkey twice; a multiset sum retains multiplicity.
- **Failure:** Identical v2 output arrives while v1 remains → sum changes from h to 2h; retirement changes it back to h → two revision changes for a supposedly unchanged support set. A8 also deletes old history, so exact past served explanations remain unavailable; Q9 must make that trade explicit.
- **Evidence:** Design A8 and §3 revision step 6; renderers §5; ADR 0009 §Ids and revision. This is arithmetic, independent of hash collisions. T4 changed the v1 revision domain (`T4:crates/rastro-core/src/claims.rs:109–110`); adopting a new tag is correct but does not solve multiplicity.
- **Fix:** Define the visible logical support set and deduplicate by rkey, or publish interpretation replacement atomically. Hash a canonical support set first; optimize only after profiling. Retain publication/version receipts if historical served answers become required.

## Agreements — checked and holds

| Point | Check / qualification |
|---|---|
| T5 cardinality correction is right | `T5:crates/rastro-extractor-github/vocabulary.toml:69,83,97` explicitly declare `one`; SDK `Predicate` at `:404–414` lacks cardinality, flattens `Definition`, and has no unknown-field denial. The fields are ignored in T5's current shape; T4 adds the matching field/default at `T4:crates/rastro-plugin-sdk/src/lib.rs:383–395`. Thus `00 §3` and semantic §5's missing-TOML-field warning are wrong. The exact merged parser was not executed; source inspection supports the correction. Strict parsing is sensible, but must be tested with the existing flattened definition shape. |
| Combined T4/T5 source has a compile defect | Read existing merged tree `6358157` with `git show`, without creating it: `claims.rs:212` binds `predicate` inside a match arm, while `:288` uses it outside. No new merge/build was run. Rework-before-merge is justified. |
| T4 replay is global and expensive | `T4:crates/rastro-storage-sqlite/src/lib.rs:581` calls refresh per batch; `lifecycle.rs:45–63` loads all supports and deletes derived intervals/links. `:96–102` correctly uses strict newer-comparable selection; `:119–141` keeps current supersession/conflict links. Preserve its tested observed-only cases. |
| Rebuild mechanism and hard-coded connector claims are accurate | `T4:lifecycle.rs:233–284` stages TEMP tables then copies in one transaction; FK string stripping is at `:256–258`. `T4:SS/src/lib.rs:601–612` constructs `Source::github` for every replay source. Fixing both is warranted. |
| T5 type/alias side effects need replacement | `T5:SS/src/lib.rs:559–588` permits retyping and inserts aliases without lifecycle removal. Supported type/alias projection is better. Rejecting every simultaneous pair of types as a conflict, however, needs compatibility/subtyping rules (A5 already proposes supertypes). |
| SQLite and standard semantics are compatible | ADR 0002/0006 permit independent WAL readers; neither concept nor RDF meaning requires a new engine. `10-models.md §Bottom line,§6` and semantic A7 support a standard profile over relational storage. No engine benchmark was run here. |
| Stable IDs, unknown valid time, OR alternatives and explicit artifacts are good foundations | ADR 0009 §Ids/§Writing; knowledge §5/§25; research `11 §Justifications and retraction`, `15 §9`. A→B→A with strictly increasing capture times works in §4a's shown sweep; tied/missing **update** clocks correctly preserve conflict. Exact byte checks prove quotation, not entailment. |
| No evidence supports the advertised scale estimates yet | Design §Unverified candidly limits measurements to DDL/query smoke checks. Its 1M-scale latency/storage thresholds are proposals, not results. I did not access the live DB to re-check `00 §4`; “no stored data anywhere” cannot follow from one owner's earlier snapshot. Preserve migration compatibility for other library consumers. |

## Questions the Owner must answer

| Question | Options / consequence |
|---|---|
| What is the approved semantic contract before implementation? | **Relational storage + published portable profile**; bespoke internal model with explicit interop debt; native RDF service with separate process/ADR changes. Engine and semantics are separate choices. |
| Which lens sets may persist, and what happens on cache miss? | All launched sets with measured cost/leases; common-set cache plus exact scoped computation; explicit incomplete/error when work budget expires. Decide acceptable registration/rebuild latency and storage at overlapping scope sets. |
| What must scope isolation prove? | Arbitrary-set replay plus hidden-input metamorphic tests including artifacts/rules; singleton-only tests as a deliberately incomplete first gate. Decide whether cross-scope undo is refused or interpreted per lens. |
| What does confirmation mean? | Independent exact proposition; occurrence-bound yes; anchor-conditioned bundle with durable dependency payload. Separately choose withdraw-yes versus explicit block, and whether newer evidence expires a yes or raises Ask. |
| How should equal-time history appear? | Preserve ordered occurrences with stable event IDs; coalesce timestamps and explicitly accept losing intermediate states. Capture-time order and source-update authority remain separate. |
| What does a merged card confirm? | Exact original member target set + group fingerprint, requiring knowledge §5 clarification; one re-keyed claim with redirects; no grouped confirmation until targeting is defined. |
| Can a vouched email define a Person forever? | Durable Person IDs with temporal identifier links; ownership-episode IDs; permanent email identity with a specified split mechanism and accepted risks. |
| What is precious and what must forgetting erase? | Export all artifacts/intents plus exact lineage with payload purging; retain only approved interpretations and explicitly weaken replay; best-effort model rerun with weaker guarantees. Amend sharing §1 together with knowledge §12. |
| When may retraction-dependent actions run? | Only after durable basis/event reconciliation exists; accept a documented interim no-action boundary. Choose proposal alternative-proof semantics and occurrence binding first. |
| Where can domain inference be replaced? | Validated derivation plugins; owner-configurable bounded rules; temporary core-only rules with an explicitly approved concept limitation. |
| Is historical service audit required? | Today's rules at a known-time cutoff; additionally retain versioned served-answer/publication receipts. Deleting retired interpretations is compatible only with the first, narrower promise unless sufficient audit input is retained elsewhere. |

## Boundaries

This is a design cross-review, not implementation approval or an engine conformance result. Main/T4/T5 code claims above were inspected; existing suite pass counts and live database contents were not rerun. SQL counterexamples establish schema/query defects, not a production latency benchmark. Only this review file was written; no decisions were made for the Owner.
