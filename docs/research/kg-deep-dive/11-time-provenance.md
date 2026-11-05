# Claims: time and provenance

Research, checked **2026-11-04**. Options and questions for the Owner; no decisions. External terms below describe other systems, not proposed glossary changes. Extends `docs/research/knowledge-graphs-2026-11.md`.

## Rastro's existing contract

| Already agreed | Repo evidence |
|---|---|
| One claim per statement, several supports; provenance, inference or confirmation as justification | `docs/decisions/knowledge.md:14–17` |
| Valid time / known time; visible conflicts; authority defaults overridable by the Owner | `docs/decisions/knowledge.md:18–20` |
| Retraction affects derived claims, pending proposals and Action runs; knowledge rebuilds from precious inputs | `docs/decisions/knowledge.md:21–26` |
| Current scope accepts deterministic structured extraction only; valid time remains `unknown` | `docs/decisions/knowledge.md:46–48` |
| Current supports determine `observed` / `superseded`; comparable newer observations win, ties/incomparability remain conflicts | `docs/decisions/knowledge.md:49` |
| Claim links, evidence-based known intervals, stable content ids, visible-state revision hash, atomic projection generations | `docs/adr/0009-claims-projection.md:10–33` |
| Paused plan specifies these contracts; temporal queries and retraction carry-through remain out of scope there | `docs/plans/2026-11-04-claims-and-about.plan.md:37–40` (D3–D6), `:47` (D13), “Out of scope” |

These are document contracts, not verification of paused T4 code. ADR 0009 supplies known-time history with unknown valid time, rather than populated bitemporal history.

## Time models and storage history

- **Valid time:** when a claim applies in the world. **Known time:** when Rastro learned/unlearned it. A late observation can describe an earlier world state.
- **Transaction/system time:** database recording time. It matches Rastro's known time only if recording, capture and interpretation coincide. ADR 0009 deliberately uses evidence `captured_at`, including on replay.
- **Unknown valid time ≠ unbounded validity.** An open-ended interval says “continues”; unknown says “bounds unavailable.” Do not silently translate one into the other.

| Model / primary source | Time, correction and supersession | Mapping / trade-off for Rastro |
|---|---|---|
| SQL:2011 [S1] | Application-time periods, system-versioned tables, or both. System history retains previous row versions; `FOR SYSTEM_TIME AS OF` queries recording history. Application periods support portion updates and temporal constraints. | Relational tables can express both axes. Temporal SQL does not choose authority or detect semantic conflict. A unique current scalar value would suppress Rastro's permitted conflicting claims; key claims/supports separately. |
| XTDB v2 [S2] | Built-in `_valid_from/to` and `_system_from/to`, closed-open intervals; user-managed valid time, system-maintained recording history; queries can select both axes. | Native late corrections and temporal joins. Still needs Rastro's support/status/authority model. Valid-time defaults must not turn unknown bounds into invented dates. Native system time would track projection writes unless precious inputs are the temporal records. |
| Datomic [S3, S4] | Datoms include entity, attribute, value, transaction and assertion/retraction operation. EAVT orders by entity/attribute/value/transaction. `as-of` reconstructs a prior database; `history` exposes assertions and retractions. | Strong audit and entity lookup model; valid time needs application attributes. Retraction removes a datom from the current view, not history. It does not itself retract all logically derived claims. Model claims as entities so contradictory values can coexist. |
| TerminusDB [S5] | Immutable commits, branches, diffs and queries at a commit. Replacement/deletion changes current branch state while retaining earlier commits. Documentation explicitly says real-world validity needs schema fields. | Useful alternative for recording interpretation changes. Commit history alone is not a second valid-time axis or a Rastro `supersedes` link. Database merge conflicts and contradictory claims are different problems. |
| Event sourcing + materialized projection [S6] | Durable inputs/change events support replay; materialized views serve reads. These are complementary, not competing approaches. | ADR 0009 already combines retained observations with a disposable projection. Observations are snapshots of fields, not necessarily domain events: polling can miss an entire close/reopen cycle. Full original retention is optional, so replay is limited to retained evidence (`docs/decisions/knowledge.md:32`). |

**Two different “as of” questions (inference):**

| Query | Inputs needed |
|---|---|
| “With today's rules, what follows from evidence captured by Tuesday?” | Evidence cutoff + current extractor/vocabulary/rules; ADR 0009 can support reconstruction. |
| “What answer did Rastro actually serve on Tuesday?” | Historical projection publication, extractor/policy versions, scope and possibly answer receipt. Rebuild under new rules can change the answer. |
| “What was the PR state Monday, according to what we knew Tuesday?” | Valid-time evidence + known-time cutoff; current `unknown` cannot answer it. |

Delayed extraction makes the distinction concrete: evidence captured at 10:00, projected at 11:00, has a known interval beginning 10:00 under ADR 0009; it was unavailable to a 10:30 reader. Keeping capture-derived known time preserves replay stability; recording publication history adds audit fidelity and storage/versioning cost. Owner choice, not a reason to silently change §25.

## Agent memory systems

| System / checked surface | Contradiction / supersession behavior | Rastro comparison |
|---|---|---|
| Graphiti v0.31.2 + Zep v4 [S7, S8] | Edge `valid_at/invalid_at` describe inferred world bounds; `created_at/expired_at` describe recording/invalidation. Graphiti deduplicates and gathers contradiction candidates using an LLM; dated overlapping candidates can be invalidated by a later valid start. Earlier-arriving knowledge need not win. | Four timestamps are useful vocabulary for distinguishing clocks. Temporal bounds are inferred, not proof. Copying automatic semantic invalidation into deterministic `observed` extraction would contradict current knowledge §23. Equal/missing dates and complete historical metadata require explicit semantics; four fields alone do not prove arbitrary historical snapshot reconstruction. |
| Mem0, current OSS migration + Platform docs [S9, S10] | OSS extraction is now single-pass ADD-only, without extraction UPDATE/DELETE. External graph integration moved out of OSS. Platform's current graph connects memories through shared entities and affects ranking; it does not assign typed relationships between entities. | Earlier UPDATE/DELETE/typed-graph descriptions are not safe descriptions of current behavior. ADD-only preserves alternatives but does not itself resolve contradictions or express `superseded`. Neither checked page specifies Rastro-style intervals, support dependencies or cascade semantics. |
| Cognee v1.6.3 / v1.6.6 [S11, S12] | v1.6.3 removes `temporal_cognify` and improves date-window search. v1.6.6 introduces durable conflict-resolution API, default conflict review and retirement of user “choices” on edits, deletes and rebuilds. | Durable resolutions resemble Owner confirmations/authority choices more than newest-wins. Temporal retrieval is not evidence of full bitemporal claim storage. Choice retirement highlights the need to bind decisions to the evidence/version reviewed; exact supersession and temporal schema remain unverified. |

No checked memory-system source establishes an equivalent of all three: quoted-span provenance, dependency-aware retraction, and a rebuildable projection preserving Owner confirmations.

## Provenance representations

| Model / primary source | What it supplies | Mapping / limitation |
|---|---|---|
| W3C PROV-O [S13] | `Entity`, `Activity`, `Agent`; `wasDerivedFrom`, usage, generation, attribution and qualified derivation; revision/invalidation relations. | Evidence and a particular claim revision can be PROV Entities; extractor run an Activity; extractor software or Owner an Agent. A support maps to qualified derivation carrying rule/span. `wasDerivedFrom` records lineage, not truth, trust class or automatic retraction. A PROV Entity is broader than Rastro's Entity. |
| Nanopublications [S14] | Assertion, assertion-provenance and publication-info named graphs, connected by a head graph. Separates the assertion's origin from publication metadata. | Good export envelope: claim content; supports/rule; exporter/creation/version. Several supports need not duplicate content. Immutable publication instances can distinguish successive revisions of one stable claim. Container structure does not define authority, intervals or supersession policy. |
| Wikidata statements [S15, S16] | Statement value + qualifiers + references + rank. Qualifiers can restrict time/context; references describe sources. `preferred`, `normal`, `deprecated` ranks influence selection. | Close to claim + supports + status, but rank is not Rastro status: normal is neutral; preferred can select current/consensus values; deprecated covers errors/outdated theories, **not correct historical values**. Historical values retain normal rank with time qualifiers. This argues for distinguishing historical succession from error/retraction. |
| RDF 1.2 reifiers [S17] | `rdf:reifies` links a resource to a triple term. Multiple reifiers may describe one proposition. Reifying an unasserted proposition does not assert it; contradictory reports can be described. Fetched Concepts spec: Candidate Recommendation Snapshot, 2026-04-07. | A stable claim IRI can reify its statement; support resources carry source-specific metadata. Avoid directly asserting every `hypothesis` as accepted world knowledge. Reification is an encoding option; RDF remains atemporal and needs an explicit temporal/status vocabulary. Engine support must be checked separately. |

**Identity issue (inference):** ADR 0009 hashes subject/predicate/object, so recurring `open` shares a claim id. A support, temporal occurrence, claim revision and claim content are consequently different identities. Wikidata restrictive qualifiers and RDF reifiers show why time/context cannot always be treated as harmless annotations. Options: keep content identity plus separate support/interval identities; or include semantic context in claim identity. The latter revises D5 and needs Owner agreement.

## Justifications and retraction

| Rastro justification | Minimum dependency record (option) | When it stops supporting a claim |
|---|---|---|
| Provenance | Evidence id + exact span + extraction rule/version + scope | Superseding observation, withdrawn support, or forgetting. Missing/unreadable data remains absence of a new observation. |
| Inference | Rule/version + **all** premise claim/revision or occurrence references | Any required premise loses support or its relevant interval ends. Recompute alternative derivations before changing conclusion status. |
| Confirmation | Durable Owner intent + target claim/revision + time + scope | Explicit withdrawal or a specified stale-confirmation rule. A new extractor must not erase the precious intent. Whether confirmation independently sustains a claim after provenance loss needs an explicit policy. |

- **JTMS:** records justifications and maintains IN/OUT belief labels for a current assumption context; dependencies explain why a conclusion is held. This is support maintenance, not a guarantee of truth or general logical consistency [S18].
- **ATMS:** labels conclusions with supporting assumption environments, retaining alternative compatible contexts and identifying inconsistent combinations. Useful when several hypotheses must coexist; labels can grow combinatorially [S19]. Contradictory sources need not force immediate global choice.
- **Rastro mapping (inference):** a justification is an AND of required premises; a claim may have an OR of independent justifications. If `J1 = A ∧ B` and `J2 = C`, retracting A invalidates J1 but the conclusion survives while J2 holds. Blanket descendant retraction would discard valid alternative support.
- Preserve the lost justification/history; distinguish `retracted` from `superseded`. Reopening a PR should end “currently closed” support, not label its previous closed interval erroneous.
- Knowledge §10 additionally requires dependency links to pending proposals and Action runs. A TMS/database cannot undo an external action: flag the affected run and propose undo as the agreed product contract requires.

| Projection maintenance option | Benefit | Cost / boundary |
|---|---|---|
| Full replay, atomically swapped generation (ADR 0009) | Simple correctness baseline; rebuilds explainable history | O(retained evidence); deterministic rules/order and retained precious inputs required. |
| Relational incremental view maintenance | Recompute affected supports/claims; dependency indexes bound cascade work | Must handle alternative proofs, cycles, authority changes and scope before limits; compare against full replay. |
| Differential dataflow [S20, S21] | Signed changes propagate through dataflows, including iterative computations; Rust implementation | Execution machinery, not an evidence/confirmation policy. Logical timestamps are not automatically valid/known time; support identities and retained replay inputs still needed. More complexity than simple local invalidation unless scale/recursive rules justify it. |

For recursive inference, raw support counts can leave a cycle self-supporting after its last external premise disappears. Options include restricting initial rules to acyclic dependencies, or using a fixed-point maintenance algorithm with a full-replay oracle. This is an algorithmic concern, not a reason to delete evidence.

## Contradiction and supersession policy options

| Policy | Strength | Failure / consequence | Rastro position |
|---|---|---|---|
| Newest comparable observation | Deterministic for structured snapshots of the same resource/field | Receipt order is unsafe for stale mirrors; item updates can be unrelated to the field. Comparability must be explicit. | Current §26 / D13; retain losers and reason. |
| Predicate-specific source authority | Tracker can outrank later chat | Wrong/stale authority can suppress correct evidence; Owner overrides must be durable/versioned. | Agreed §9, wider authority overrides deferred by paused plan. |
| Keep conflicting claims visible; Ask | Honest under ties, incomparable clocks or uncertain extraction | More unresolved cases for retrieval/Owner | Agreed §8; current tie/incomparability behavior. |
| Rank accepted view separately from recorded reports | Keeps all source statements while selecting current answers | More state than a single status field; selection policy must be reproducible | Wikidata analogy, possible future alternative; not agreed replacement. |

`superseded` means different things across sources: closed system period (SQL/XTDB), datom retraction (Datomic), prior commit version (TerminusDB), expired edge (Graphiti), publication/revision metadata, or editorial rank (Wikidata). None automatically supplies Rastro's explicit `supersedes`/`conflicts_with` links plus justification lifecycle.

## Concrete example: one PR, two sources, open → closed → open

Illustrative, all one visible scope. Sources A/B observe the **same canonical GitHub PR field**; their `source_updated_at` values are assumed comparable. Times on 2026-11-04 UTC. O = hash(PR, `code:state`, `open`); C = hash(PR, `code:state`, `closed`). Capture times define known intervals; source update times only select the current observation, not valid bounds.

| Captured | Source / state / item update | Result under D13 |
|---|---|---|
| 09:00 | A / open / 08:55 | O `observed`; first interval opens 09:00. |
| 09:02 | B / open / 08:55 | Same O, second support; revision changes. |
| 10:05 | A / closed / 10:00 | Newer comparable observation: C `observed`; O `superseded`, interval closes 10:05; C supersedes O. B's old report stays in history. |
| 10:10 | B / closed / 10:00 | Same C, second support; revision changes. |
| 11:05 | B / open / 11:00 | O `observed` again, same id, new support/revision/interval; C `superseded`; O supersedes C. |
| 11:10 | A / open / 11:00 | Same O, another current support/revision. |

- Known intervals: O `[09:00,10:05)` and `[11:05,∞)`; C `[10:05,11:05)`. Valid time stays `unknown` throughout. The observation sequence is not proof the PR closed exactly at 10:00 or reopened at 11:00.
- Variant: at 10:05 A says closed while B says open with the **same** comparable update time; both remain `observed`, linked `conflicts_with`. Different known intervals may overlap. A missing/null response closes neither.
- Retain the 10:05 replacement and 11:05 replacement as distinct transitions. If every historical content-level `supersedes` link is accumulated, O↔C forms a cycle; if links show current replacement only, historical explanation needs interval/transition context. ADR 0009 names the links but does not fully specify this distinction.
- A claim's presence interval does not capture every support revision: O gains support at 09:02 without starting a new presence interval. Historical `why` needs support/link/status history or deterministic replay to the cutoff.

## Gaps / alternatives for Owner review

All below are research inferences, not accepted changes.

| Question exposed by literature/example | Options / trade-off |
|---|---|
| Capture-derived knowledge vs actual served history | Keep capture semantics and document it; optionally retain publication/audit history separately. Native database transaction time cannot silently substitute for §25. |
| Unknown valid bounds, late events and corrections | Explicit unknown representation now; later add evidence-backed bounds and known-time versions of corrections. Ordinary unbounded periods would overstate knowledge. |
| What makes observations comparable? | Same upstream resource/version clock; authority policy for unrelated sources; otherwise conflict. Specify missing timestamps, clock skew and equal-time behavior. |
| Repeated statement identity and supersession | Stable content ids plus occurrence/transition records; alternatively contextual claim ids. Historical links must not imply permanent logical ordering. |
| Revision hash vs historical version | Hash is a visible-state fingerprint, not chronological sequence or retained snapshot. Same visible state can recur; version inputs and retain historical derivations if audit requires them. |
| Provenance-only table vs all justification kinds | Extend support records or introduce typed justification/premise records. Preserve exact spans; multiple premises form one derivation, not independent votes. |
| Retraction and confirmation semantics | Define alternative-support survival, explicit negative Owner correction, stale confirmation targeting, cycles and dependent proposals/actions before implementing cascade. |
| Rebuild under changed rules | Recompute today's interpretation; retain old run/version/answer receipts if reconstructing former decisions matters. LLM re-extraction is not deterministic unless outputs/configuration are retained. |
| Scope-aware maintenance | ADR 0009 already filters all derived metadata. Apply the same rule to historical supports, authority resolutions and inference paths; a hidden premise/replacement must not silently determine a visible answer. |

No engine choice follows from these distinctions. Leaning intentionally omitted: the Owner first needs to choose historical-query and correction semantics.

## Sources

All URLs checked **2026-11-04**; vendor behavior describes the checked surface, not a benchmark or operational guarantee.

| ID | Primary source |
|---|---|
| S1 | Kulkarni & Michels, *Temporal Features in SQL:2011*, SIGMOD Record, 2012: https://sigmodrecord.org/publications/sigmodRecord/1209/pdfs/07.industry.kulkarni.pdf (authors' account, not the purchased ISO text). |
| S2 | XTDB v2 concepts / SQL: https://docs.xtdb.com/concepts/key-concepts.html ; https://docs.xtdb.com/quickstart/sql-overview.html |
| S3 | Datomic index model: https://docs.datomic.com/indexes/index-model.html |
| S4 | Datomic history tutorial: https://docs.datomic.com/client-tutorial/history.html |
| S5 | TerminusDB temporal-memory distinction: https://terminusdb.com/docs/knowledge-graph-for-ai-agents/ ; commit queries: https://terminusdb.org/docs/time-travel-howto/ |
| S6 | Martin Fowler, *Event Sourcing*, 2005: https://martinfowler.com/eaaDev/EventSourcing.html |
| S7 | Zep v4 fact timestamps: https://help.getzep.com/v4/facts |
| S8 | Graphiti pinned v0.31.2, `_apply_temporal_invalidation`, `resolve_edge_contradictions`, `_dedupe_extracted_edge`: https://github.com/getzep/graphiti/blob/v0.31.2/graphiti_core/utils/maintenance/edge_operations.py |
| S9 | Mem0 OSS migration: https://docs.mem0.ai/migration/oss-v2-to-v3 |
| S10 | Mem0 current Platform graph: https://docs.mem0.ai/platform/features/graph-memory |
| S11 | Cognee v1.6.3 release, 2026-10-07: https://github.com/topoteretes/cognee/releases/tag/v1.6.3 |
| S12 | Cognee v1.6.6 release, 2026-10-29: https://github.com/topoteretes/cognee/releases/tag/v1.6.6 |
| S13 | W3C PROV-O Recommendation, 2013-04-30, §§3.1–3.3: https://www.w3.org/TR/prov-o/ |
| S14 | Nanopublication Guidelines, working draft: https://nanopub.net/guidelines/working_draft/ |
| S15 | Wikidata data model: https://www.wikidata.org/wiki/Wikidata:Data_model |
| S16 | Wikidata ranking guidance: https://www.wikidata.org/wiki/Help:Ranking |
| S17 | W3C RDF 1.2 Concepts, fetched CR Snapshot 2026-04-07, §§1.5–1.6: https://www.w3.org/TR/rdf12-concepts/ |
| S18 | Doyle, *A Truth Maintenance System*, 1979: https://www.sciencedirect.com/science/article/pii/0004370279900080 ; author's clarification, *The Ins and Outs of Reason Maintenance*, IJCAI 1983: https://ijcai.org/proceedings/1983/82 |
| S19 | Reiter & de Kleer, *Foundations of Assumption-based Truth Maintenance Systems*, AAAI 1987: https://cdn.aaai.org/AAAI/1987/AAAI87-033.pdf ; de Kleer & Williams, ATMS alternatives/control, AAAI 1986: https://mlanthology.org/aaai/1986/dekleer1986aaai-back/ |
| S20 | Differential dataflow implementation and signed-update example: https://github.com/TimelyDataflow/differential-dataflow |
| S21 | McSherry et al., *Differential Dataflow*, paper distributed with implementation: https://docs.rs/crate/differential-dataflow/0.10.0/source/differentialdataflow.pdf |

## Unverified

- No database/framework installed or benchmarked; no claim of drop-in fit, Rust embedding, or complete export fidelity.
- Exact Mem0 package release introducing the new algorithm; graph pages establish current migration behavior, not all hosted conflict-resolution internals.
- Cognee exact persisted temporal/choice schema, dependency cascade and historical “as of” behavior. Release notes verify features, not implementation completeness. `temporal_cognify` removal is verified, superseding the earlier research uncertainty.
- Graphiti/Zep immutable histories of every changed edge attribute, complete equal/missing-date policy across paths, and equivalence between hosted Zep and pinned OSS code. Inspected invalidation code does not prove these.
- RDF 1.2 Recommendation advancement after the fetched 2026-04-07 CR snapshot; use the fetched status rather than assuming final standardization. Engine support for reifiers/annotation syntax was not evaluated.
- SQL:2011 normative text and cross-vendor conformance; S1 is its authors' public technical explanation.
- Historical scopes, authority changes, confirmations, and retraction propagation in Rastro's future implementation; ADR 0009 contracts were reviewed, paused branches were not inspected or changed.
