# Semantic architecture: identified claims, standard meaning, local storage

2026-11-05 · `arch-semantic` · **Proposal only; every choice belongs to the Owner.**
Independent design; `21-arch-pragmatic.md` was not read. Research references below are siblings in this folder. New `rc:` terms and SQL names are proposed interfaces, not additions to the glossary.

## 1. Design summary

- **Leaning:** RDF 1.2 claim semantics over typed SQLite tables; no persistent second engine.
- Each Claim is an IRI reifier of one proposition; never equate a report with an asserted world statement.
- Evidence stays immutable. Separate Justifications encode provenance, inference and Confirmation.
- PROV-O describes lineage; Rastro rules determine Status, authority and retraction.
- Nanopublication-style packages carry one claim revision, its supports and publication metadata.
- SHACL-style validation rejects malformed input; contextual cardinality reports conflicts without rejecting claims.
- SKOS supplies definition/example language; installed vocabulary versions remain authoritative.
- Capture-derived known time, explicit unknown valid time, occurrence-specific replacement records.
- Stable Entity/Claim IDs survive merges; scoped identity overlays preserve original references.
- Accounts link to Persons; accounts never become humans through equality reasoning.
- Scope-filter all justifications before computing identity, Status, revision, links or limits.
- Fixed MCP operations remain; a bounded expansion operation is a separate proposal.
- Rebuild uses one SQLite transaction boundary; future stochastic extraction needs durable artifacts.
- No OWL equality/functional-property reasoning; no required SPARQL or RDF 1.2 parser on the read path.

## 2. Areas A1–A11

### A1 — Data model

| | Proposal / trade-off |
|---|---|
| Proposal | One binary Claim reifier per exact `(subject, predicate, typed object)`. A Commitment is an Entity with several claims, not a hyperedge or an opaque JSON object. Supports, intervals and revisions have separate identities. Unasserted reification describes hypotheses and contradictory reports without endorsing either. A scoped accepted-view export may assert current observed/confirmed propositions in a separate named graph; the default graph contains no world assertions. |
| Rejected | Edge-only LPG cannot receive links to the edge; bare RDF triples lose the Claim identity; OWL functional properties turn retained contradictions into inconsistency/equality. A graph per support duplicates content. |
| Costs | More nodes than triples alone; custom roll-up still required. SQLite implements this restricted RDF profile, **not** arbitrary RDF datasets or SPARQL. RDF 1.2 syntax remains provisional; export also offers RDF 1.1 statement nodes. |
| Agreements | Keeps concept §1, knowledge §1/§5, ADR 0009 §Ids and revision. **Amends ADR 0009 §Tables:** type becomes a supported claim with a vocabulary-IRI object (a third object category alongside Entity and literal); entity type column becomes a derived convenience. RDF typed-literal equality does not replace existing byte-level Claim identity. |
| Owner question | Adopt standard semantic identities with a relational implementation, or require a native RDF dataset now? Q1. |

Basis: `10-models.md §1. RDF (triples, quads, RDF 1.2)`, `§3. Others`; `11-time-provenance.md §Provenance representations`.

### A2 — Claim reification and provenance

| | Proposal / trade-off |
|---|---|
| Proposal | Claim → Justification → exact Evidence spans; inference has an AND set of premise Claim occurrences, several justifications are OR alternatives. Each provenance justification records extractor/rule/vocabulary versions and run. Map a **revision**, not the eternal proposition, to `prov:Entity`; extractor run to `prov:Activity`; software/Owner to `prov:Agent`. A qualified derivation points to the exact source Entity and records span metadata. |
| Packaging | One immutable package per `(claim, scoped revision, export profile)`: head, assertion, provenance, publication-info graphs. Assertion graph contains the **reified report**, including Status; optional accepted-world graph is separate. Supports are individually identified qualified derivations, so several sources need no duplicated claim. This is nanopublication-style, not a claim of full ecosystem/trusty-URI conformance. A Share is ingested as Evidence from its sender, never as locally confirmed knowledge. |
| Rejected | A list of Evidence IDs cannot represent exact spans or AND dependencies. `prov:wasDerivedFrom` alone neither verifies truth nor propagates retractions. Hashing an entire package into the Claim ID breaks stable references when support changes. |
| Costs | Additional tables/resources and an export profile. Package hash and Rastro revision hash serve different purposes; a redacted Share cannot retain the hash of its unredacted body. |
| Agreements | Keeps knowledge §5/§6/§10, sharing §1–§6, renderers §5; extends ADR 0009 provenance-only supports to all glossary Justification kinds. No concept contradiction. |
| Owner question | Make standard provenance packages part of portable export now, or define the mapping now and ship serialization later? Q8. |

Basis: `11-time-provenance.md §Provenance representations` and `§Justifications and retraction`; `10-models.md §6. Interop costs`.

### A3 — Temporal model

| | Proposal / trade-off |
|---|---|
| Proposal | Half-open known intervals on justification occurrences, plus unioned Claim-presence intervals. A closed interval and later recurrence are different rows. Replacements reference **justifications/occurrences**, not permanent ordering between propositions. `supersedes` in current `why` is the current transition; historical transitions remain inspectable. Explicit `valid_kind=unknown` until Evidence supplies bounds; never infer bounds from item update time. |
| Queries | Future `as_of_known` means today's pinned interpretation of inputs captured by cutoff. Owner inputs use their own durable recording time. Actual answers formerly served require separate publication receipts; not promised by these intervals. Evidence arriving late uses its retained capture time, not rebuild wall time. |
| Rejected | Engine transaction time resets on rebuild. One interval per Claim cannot represent open→closed→open. Accumulated O↔C content-level links falsely look like a timeless supersession cycle. Treating unknown as `[-∞,+∞)` invents validity. |
| Costs | Temporal joins and interval coalescing; support/revision history exceeds presence history. Incremental processing must equal full replay, including equal timestamps ordered by Evidence ID. |
| Agreements | Keeps knowledge §7/§25/§26, plan D5/D6/D13, ADR 0005. Amends ADR 0009 §Tables to attach historical transitions to occurrences; extends unknown-only valid time later, not in T4. |
| Owner question | Is capture-cutoff reconstruction sufficient, or must Rastro reproduce the exact answer formerly served? Q6. |

Basis: `11-time-provenance.md §Time models and storage history` and `§Concrete example: one PR, two sources, open → closed → open`.

### A4 — Identity and merges

| | Proposal / trade-off |
|---|---|
| Proposal | IRIs globally name Entities; exact canonical identifiers retain D7 normalization. Preserve `e_…`/`c_…` hashes and expose `urn:rastro:entity:e_…` / `urn:rastro:claim:c_…`. Prefix expansion for existing predicate keys is a published, injective registry; retain their v1 hash inputs. New terms receive canonical IRIs. No blank nodes in durable references. Global naming enables matching, not authorization or confidentiality. |
| Merges | A symmetric, reified `rc:merge` Claim joins **same-kind** Entities; normalize endpoints before hashing. Supported active merges form scoped connected components; choose minimum visible member ID as display representative. Original identifiers always resolve and original Claim IDs never change. Merge undo is a durable Owner rule blocking that merge until explicit reversal, rather than a transient deletion. Recompute the affected component; union-find alone cannot split. Conflicting cannot-link/merge paths cause Ask, not an arbitrary cut. |
| Persons | Accounts link to a Person through proposed `rc:heldBy`. A Person may be minted from a durable Owner intent ID. Merge duplicate Persons; do not assert GitHub account = Slack account = Person. Shared accounts may have several holders. Source-vouched exact keys alone can automate identity; names only raise candidates. |
| Dedup | Equal exact triples retain one Claim/many supports. Claims equal only after a Merge remain separate immutable Claims, grouped in the read result with all member Claim IDs and a group fingerprint. Never pass that fingerprint as a renderer Claim ID; confirmation requires choosing original targets. |
| Rejected | `owl:sameAs` means unrestricted substitution; withdrawing it plus recomputation is possible, but it supplies neither scoped blocking rules nor reversible Rastro policy. Destructive merges/re-keying break renderer targets. `skos:exactMatch` is vocabulary mapping, not human identity. |
| Costs | Scope-aware connectivity and grouping. Display representative may change when visible membership changes. Extra entity/claim for each account→Person link. |
| Agreements | Keeps concept §1's one identity as one resolved view; keeps renderers §5 and ADR 0009 IDs. **Explicit amendment to knowledge §5:** one Claim per exact statement; merged-subject equivalents are grouped with original IDs, not collapsed into one Claim. Literal account→Person merging would contradict vocabulary §12 and glossary Person; this proposal rejects it. Keep knowledge §30's rename limit. |
| Owner question | Accept overlay/grouping and Person links, or accept ID remapping to obtain one materialized Claim after merging? Q2. |

Basis: `13-identity.md §3. Merge strategies`, `§4. Undo and rebuild semantics (C or D)`, `§6. Accounts vs Persons`, `§8. Scopes, privacy and forgetting`.

### A5 — Vocabulary and schema

| | Proposal / trade-off |
|---|---|
| Proposal | Compile installed TOML vocabulary into a versioned semantic manifest: IRI, `skos:definition`, `skos:scopeNote`, `skos:example`, custom counter-example/evidence requirements, endpoint constraints and policy. Core meta-model separate from standard/extension packages. Resolve dependencies at pinned versions; allow declared cross-namespace subtypes. Keep definitions stable under one term IRI; changed meaning needs a new IRI and reviewed mapping. |
| Validation | Core enforces a small SHACL-style profile: exact one subject/predicate/object, legal object kind/datatype, declared vocabulary, supported endpoint types, span integrity and grants. Validate a candidate bundle atomically so submitted type claims can establish endpoint types independent of message order. Type dependencies become justification premises. Cardinality applies to a visible subject **and observation context**; emit conflict diagnostics with Warning severity, never reject those claims. |
| Mapping | SKOS broader/narrower/exact-match language describes proposed term mappings; mapping application requires a confirmed mapping Claim, version-specific scope and provenance. No automatic OWL equivalence or silently rewritten predicate IDs. |
| Rejected | `rdfs:domain/range` infer types; they are not validation. Global `sh:maxCount 1` rejects legitimate contradictory reports. Full SHACL/SPARQL extensions and dynamic OWL reasoners enlarge the core and query attack surface. Model-generated schema cannot install itself. |
| Costs | Profile/compiler conformance work and deterministic diagnostics. Typed lexical values need a registry; preserve lexical identity (`"01"^^integer` differs from `"1"^^integer` in Claim hashing). A changed definition forces re-extraction, not just structural revalidation. |
| Agreements | Keeps vocabulary §1–§10/§13 and plugins §1/§4; amends ADR 0008 §Core validation for multi-vocabulary/bundle validation. Existing deterministic observed gate stays. SHACL 1.0-compatible constraints first; no dependency on SHACL 1.2 draft features. |
| Owner question | Restricted standard-aligned validation profile, or a complete third-party SHACL implementation? Q7. |

Basis: `10-models.md §4. Rastro's needs mapped to each model`; `15-llm-ontology.md §2. Ontology-guided extraction`, `§4. Validation, repair and confidence`, `§5. Learning and evolving a vocabulary`.

### A6 — Traversal and indexes

| | Proposal / trade-off |
|---|---|
| Proposal | Indexed binary Claim adjacency, both directions; Entity→Entity claims count as one semantic hop, despite physical joins via the reifier. Literal claims are attributes. Evidence/provenance/type edges do not become semantic shortcuts. Search retains raw hits, maps them through supports to Entity seeds, then bounded one/two-hop expansion. `about` stays exactly one hop with D15 ordering. |
| Indexes | `(generation,subject,predicate,id)`, inverse Entity object; justification `(generation,claim,id)`; span→Evidence and reverse; premise→dependent; alias value/kind/Entity; interval lookup by justification/start/end. Scope→Evidence index aids early filtering. Sort on computed visible known-time starts; a global timestamp index cannot supply scoped order safely. |
| Rejected | Closure tables can be quadratic and erase paths; unrestricted property paths/recursive SQL do not bound work. PPR/community summaries add privacy and relevance policy without proving justification. |
| Costs | Several B-trees, write amplification, scoped roll-up before sorting. Proposed expansion budgets: 5 seeds, depth ≤2, 25 eligible neighbors/entity, 2,000 examined candidates, 50 output claims, 64 KiB response; stable ID ties and explicit incomplete reasons. These need measurement and owner approval, not performance promises. |
| Agreements | Keeps knowledge §15, surfaces §23 and plan D15. New expansion is separate from `about`; no concept contradiction. |
| Owner question | Fixed APIs only initially, or add bounded expansion once its equal-token evaluation passes? Q5. |

Basis: `14-retrieval.md §Traversal and ranking experiments`, `§Index design: relational versus native graph`; `12-engines.md §Personal-scale fit: 10k–1M evidence rows, 100k–10M claims`.

### A7 — Store and engine

| | Proposal / trade-off |
|---|---|
| Proposal | Keep SQLite WAL + FTS5 behind domain traits. Typed tables are the authoritative implementation of the RDF profile; serializer produces the equivalent dataset. Build generation-keyed projection tables in the same database. Under writer lock, replay against pinned input high-water marks and version manifest; finish indexes/validation, then atomically activate generation + checkpoints. Each reader pins one transaction/generation; retired generations are writer-collected after readers finish. Failed rebuild preserves active generation. |
| Rejected | Oxigraph offers native RDF/SPARQL and useful query tooling, but its documented cross-process read-only opening beside a writer is undefined behavior. Sharing a Store among threads does not fix separate CLI/MCP processes. A daemon changes ADR 0006; immutable Oxigraph copies add dual-store activation, disk and forgetting hazards. Generic quad-only SQLite loses domain integrity and adds joins to every hot read. |
| Costs | Custom profile-to-RDF adapter, no arbitrary SPARQL. Generation-keyed storage temporarily doubles projection disk; WAL growth under long reads needs measurement. Read-time scope roll-up is allowed; extraction/maintenance remain write-only. |
| Agreements | Keeps ADR 0001/0002/0006 and ADR 0009 §Writing; amends ADR 0009 physical schema. No engine/process contradiction to concept §5. Correct ADR 0002's historical engine-date rationale when next amended (`12-engines.md §Constraints and important corrections`). |
| Owner question | Prioritize separate local readers (leaning), or authorize a single storage-owning service for native SPARQL? Q1. |

Basis: `12-engines.md §Transactions, retrieval, time and rebuild`, `§Hybrid and rebuild patterns (proposals)`. Oxigraph limitation independently checked 2026-11-05: [Store API](https://docs.rs/oxigraph/0.5.13/oxigraph/store/struct.Store.html#method.open_read_only).

### A8 — Several extractors on one Entity

| | Proposal / trade-off |
|---|---|
| Proposal | Explicit config list of extractor bindings per source; retain connector default as one list member. Checkpoint by binding + extractor/version + vocabulary hash + source/scope. Identical propositions deduplicate; justifications retain each producing rule. Explicitly declared interpretation slots retire an older extractor version's output only after complete successful replay; failed upgrades keep last good output. |
| Authority | Compare evidence about the same normalized resource/field under a declared clock domain. A greater upstream item-update timestamp can order snapshots of that field, **not** establish valid time. Ties/missing clocks/incomparable domains preserve conflicts. Across unrelated sources apply pinned predicate authority rules or Ask. Several extractors of the same Evidence are interpretations, not independent corroboration. Hidden replacements do not retire visible supports in a scoped result. |
| Rejected | Newest extractor version wins globally; source arrival order; old-version output retained as independent current votes; discovery of all installed extractors. |
| Costs | More checkpoints, replay barriers and authority metadata. Interpretation slot declarations must distinguish “same rule, upgraded” from genuinely alternative rules. |
| Agreements | Keeps knowledge §5/§8/§9/§26; **amends ADR 0008 §Binding and plugins §15** from singular binding to explicit lists. Future authority overrides extend the paused scope; no change is silently folded into T4. |
| Owner question | Allow several explicitly bound extractors now, or design the slot contract but postpone list execution? Q9. |

Basis: `00-current-state.md §2b. Host and validation`, `§5. Decided vs open, per area`; `11-time-provenance.md §Contradiction and supersession policy options`.

### A9 — Hypothesis, Confirmation and retraction

| | Proposal / trade-off |
|---|---|
| Proposal | Preserve deterministic structured→observed gate. New hypothesis-only path accepts text-derived bundles with exact spans, fixed vocabulary and quarantined permissions. Missing required endpoint/type/identity information remains a rejected/partial candidate in an extraction artifact, not an invented Claim. Model confidence never sets Status. Owner confirmation stores the exact accepted proposition bundle, reviewed scoped revisions, interpretation hashes and durable intent time outside projection. |
| Replay | Record stochastic extraction artifacts durably: ordered inputs/hashes/retained context, prompt/model/settings/adapter/rule/vocabulary hashes, raw output, parsed candidates, repairs and validation results. Rebuild replays recorded submissions; re-extraction creates a new artifact. New interpretations never inherit old confirmations automatically. Deterministic extraction can be replayed from code + retained Evidence. |
| Retraction | Explicit retract intent installs a scoped durable blocking rule against the target proposition until reversed. This defeats rediscovery of the same hypothesis on replay. Undoing only a confirmation removes that yes but allows other justifications to sustain the Claim. A positive confirmation independently sustains the proposition after its model derivation changes; original reviewed content remains retained. Forgetting erases dependent artifacts/confirmation payloads as required and leaves a non-content marker. |
| Maintenance | Justification live iff all required premises live and its time/policy gates hold; Claim survives any alternative justification. Explicit target block wins over those alternatives. Lost last derivation gives `retracted`; ordinary comparable observation replacement gives `superseded`; surviving hypotheses give `hypothesis`; surviving confirmation gives `confirmed`; otherwise structured provenance gives `observed`. Inference-only conclusions remain `hypothesis` unless a deterministic rule with observed/confirmed premises qualifies them. Cyclic inference rejected initially; merge connectivity is a separate non-inferential computation. |
| Carry-through | Dependency index reaches derived claims, pending Proposals and Action runs. Withdraw unsupported Proposals; flag affected runs and propose undo. Never imply the database reversed an external action. |
| Rejected | Calling an LLM again during rebuild is not deterministic. A disposable cache cannot contain unique interpretation inputs. Transitive blanket deletion discards independent proofs. Counting supports can leave a cycle self-supporting. Ordinary Owner prose is not a Confirmation. |
| Costs | More precious input, retention/export/privacy obligations; confirmation targeting requires a scope fingerprint. Negation, entailment and correct Person resolution remain fallible even with valid SHACL/span checks. |
| Agreements | Keeps concept §1/§2/§7, knowledge §6/§10/§13/§14, levels §2/§9/§10, renderers §5. **Explicit contradiction requiring amendment: knowledge §12 says evidence, confirmations, rules and outcomes are the only precious inputs; this design adds extraction artifacts.** Amend sharing §1 accordingly. Hypothesis acceptance also replaces the current-only restriction in knowledge §23/ADR 0008; not authorized within current T4/T5. |
| Owner question | Preserve exact stochastic interpretations as new precious inputs, or accept best-effort re-extraction and weaker replay guarantees? Q3. Also choose confirmation/retraction semantics, Q4. |

Basis: `15-llm-ontology.md §3. Grounding: “no span, no claim”`, `§8. Security: evidence and ontology poisoning`, `§9. Determinism and replay`; `11-time-provenance.md §Justifications and retraction`.

### A10 — Retrieval for agents

| | Proposal / trade-off |
|---|---|
| Proposal | Catalog-defined `search`, `coverage`, `about`, `why`; same JSON over CLI/MCP. Entity IRIs/legacy IDs/exact canonical identifiers accepted; bare names remain search candidates. Include predicate definition/version references. No query-time extraction or network vocabulary fetch. `why` returns bounded justification records and premise Claim IDs; follow-up calls walk inference, with depth/work caps if bundled later. |
| Scope | Launch scopes are trusted context, not tool arguments. Construct visible justification graph first; an AND derivation with any hidden premise is unavailable. Recompute intervals, transitions, types, identity groups, aliases, counts and revisions. A scoped revision includes visible justification IDs, premise occurrence refs, policy/term hashes and links; no hidden counts or package hashes. Owner intents compare the scoped revision under the same scope context. |
| Completeness | Preserve D15 order/newest interval-start semantics and 20/default, 50/max. Report source Coverage, extraction state, retrieval limits and excerpt clipping separately. Unknown/hidden-only `about` both empty; unknown/hidden-only `why` same error. No silent global degree/ranking leakage beyond already accepted FTS statistics. |
| Rejected | Raw SPARQL/SQL/Cypher on MCP lets callers omit scope/provenance joins and creates unbounded path/federation work. Endpoint connectivity is not an inference proof. Native RDF does not require exposing SPARQL. |
| Costs | Repeated tool calls; more response metadata. Revision v2 must include newly visible temporal/identity/justification fields; this **amends ADR 0009 §Ids and revision**, preserving Claim IDs. |
| Agreements | Keeps surfaces §22–§26, ADR 0006/0007, plan D15–D19; optional expansion requires a new catalog decision. |
| Owner question | Approve fixed operations with compact vocabulary context; defer a query language? Q5. |

Basis: `14-retrieval.md §Agent query surface: options, not a decision`; `15-llm-ontology.md §7. Vocabulary as query-time context`.

### A11 — Fate of T4/T5

| | Proposal / trade-off |
|---|---|
| Proposal | **Rework both; preserve useful code/tests.** T4 contains lifecycle/rebuild fixtures; T5 contains GitHub extraction rules, required vocabulary language and author handling. Neither branch is a safe foundation to merge unchanged. See §5 for exact disposition. |
| Rejected | Drop everything for a triplestore loses valid work and process guarantees. Merge first then retrofit semantics bakes in mutable types, unscoped global lifecycle and incomplete recurrence history. |
| Costs | Storage/core changes overlap; resolve interfaces together before implementation. Existing tests passing on each branch do not establish combined correctness. |
| Agreements | Keeps paused plan's owner gate and D1–D24 where identified above. Extending future identity/hypothesis/retraction design does **not** authorize implementation beyond resumed assigned scope. |
| Owner question | Resume minimal observed-claim work with these seams, or replace the paused plan after approving the complete model? Q10. |

Basis: `00-current-state.md §3. Unmerged branches (both from 1fb2108)`; paused plan §Pause.

## 3. Concrete schema and RDF profile

**Design DDL, not a migration.** Standalone sketch validated with SQLite; existing Evidence/runs/Coverage/config remain unchanged. `evidence_ref` models an ID bridge to existing integer Evidence IDs, not another copy of observations. `owner_event`, `confirmation` and `artifact` are durable inputs; everything generation-keyed is disposable. Hashes use tagged, length-prefixed canonical bytes; no hashing SQL serialization or arbitrary Turtle order. New versioned encodings require published golden fixtures. `rc:` below abbreviates a proposed namespace, not a deployed domain commitment.

```sql
PRAGMA foreign_keys=ON;
CREATE TABLE evidence_ref (
  iri TEXT PRIMARY KEY, local_evidence_id INTEGER UNIQUE NOT NULL,
  scope TEXT NOT NULL, content_hash TEXT NOT NULL
);
CREATE TABLE owner_event (
  iri TEXT PRIMARY KEY, kind TEXT NOT NULL
    CHECK(kind IN ('confirm','retract','undo_confirmation','block_merge','reverse_rule')),
  scope TEXT NOT NULL, recorded_at TEXT NOT NULL, target_claim TEXT NOT NULL,
  seen_revision TEXT NOT NULL, scope_fingerprint TEXT NOT NULL,
  proposition_bundle TEXT NOT NULL, interpretation_hash TEXT NOT NULL,
  reverses TEXT REFERENCES owner_event(iri)
);
CREATE TABLE confirmation (
  iri TEXT PRIMARY KEY REFERENCES owner_event(iri), payload_hash TEXT NOT NULL
);
CREATE TABLE artifact (
  iri TEXT PRIMARY KEY, scope TEXT NOT NULL, recorded_at TEXT NOT NULL,
  extractor TEXT NOT NULL, extractor_version TEXT NOT NULL, rule_version TEXT NOT NULL,
  vocabulary_hash TEXT NOT NULL, prompt_hash TEXT NOT NULL,
  model_revision TEXT NOT NULL, settings_json TEXT NOT NULL,
  adapter_version TEXT NOT NULL, validator_version TEXT NOT NULL,
  request_context TEXT NOT NULL, raw_response TEXT NOT NULL,
  parsed_candidates TEXT NOT NULL, repairs_and_diagnostics TEXT NOT NULL
);
CREATE TABLE artifact_input (
  artifact TEXT REFERENCES artifact(iri), ordinal INTEGER NOT NULL,
  evidence TEXT NOT NULL REFERENCES evidence_ref(iri), bytes_hash TEXT NOT NULL,
  PRIMARY KEY(artifact,ordinal)
);
CREATE TABLE generation (
  id INTEGER PRIMARY KEY, input_high_water TEXT NOT NULL,
  versions_manifest TEXT NOT NULL, published_at TEXT
);
CREATE TABLE active_generation (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  generation INTEGER NOT NULL REFERENCES generation(id)
);
CREATE TABLE entity (
  generation INTEGER REFERENCES generation(id), iri TEXT NOT NULL,
  canonical_identifier TEXT NOT NULL, PRIMARY KEY(generation,iri),
  UNIQUE(generation,canonical_identifier)
);
CREATE TABLE vocabulary_term (
  generation INTEGER REFERENCES generation(id), iri TEXT NOT NULL,
  version TEXT NOT NULL, package_hash TEXT NOT NULL, kind TEXT NOT NULL,
  definition TEXT NOT NULL, not_to_confuse_with TEXT NOT NULL,
  examples_json TEXT NOT NULL, counter_examples_json TEXT NOT NULL,
  evidence_requirement TEXT NOT NULL, domain_json TEXT NOT NULL,
  range_json TEXT NOT NULL, parent_types_json TEXT NOT NULL,
  cardinality TEXT NOT NULL CHECK(cardinality IN ('one','many')),
  freshness TEXT NOT NULL, authority TEXT NOT NULL, sensitivity TEXT NOT NULL,
  PRIMARY KEY(generation,iri,version)
);
CREATE TABLE claim (
  generation INTEGER NOT NULL, iri TEXT NOT NULL, subject TEXT NOT NULL,
  predicate TEXT NOT NULL, object_entity TEXT, object_term TEXT, object_lexical TEXT,
  object_datatype TEXT, object_language TEXT,
  PRIMARY KEY(generation,iri),
  FOREIGN KEY(generation,subject) REFERENCES entity(generation,iri),
  FOREIGN KEY(generation,object_entity) REFERENCES entity(generation,iri),
  CHECK((object_entity IS NOT NULL AND object_term IS NULL AND object_lexical IS NULL
         AND object_datatype IS NULL AND object_language IS NULL)
     OR (object_entity IS NULL AND object_term IS NULL AND object_lexical IS NOT NULL
         AND object_datatype IS NOT NULL)
     OR (object_entity IS NULL AND object_term IS NOT NULL AND object_lexical IS NULL
         AND object_datatype IS NULL AND object_language IS NULL))
);
CREATE TABLE justification (
  generation INTEGER NOT NULL, iri TEXT NOT NULL, claim TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('provenance','inference','confirmation')),
  extractor TEXT, extractor_version TEXT, rule_version TEXT NOT NULL,
  vocabulary_hash TEXT NOT NULL, run_id INTEGER, binding TEXT,
  interpretation_slot TEXT, clock_domain TEXT, source_updated_at TEXT,
  artifact TEXT REFERENCES artifact(iri),
  confirmation TEXT REFERENCES confirmation(iri),
  PRIMARY KEY(generation,iri),
  FOREIGN KEY(generation,claim) REFERENCES claim(generation,iri),
  CHECK((kind='confirmation')=(confirmation IS NOT NULL))
);
CREATE TABLE support_span (
  generation INTEGER NOT NULL, justification TEXT NOT NULL,
  evidence TEXT NOT NULL REFERENCES evidence_ref(iri), field TEXT NOT NULL,
  byte_start INTEGER NOT NULL CHECK(byte_start>=0),
  byte_end INTEGER NOT NULL CHECK(byte_end>byte_start), quote TEXT NOT NULL,
  PRIMARY KEY(generation,justification,evidence,field,byte_start,byte_end),
  FOREIGN KEY(generation,justification) REFERENCES justification(generation,iri),
  CHECK(length(CAST(quote AS BLOB))=byte_end-byte_start)
);
CREATE TABLE known_interval (
  generation INTEGER NOT NULL, iri TEXT NOT NULL,
  justification TEXT NOT NULL, start TEXT NOT NULL, end TEXT,
  PRIMARY KEY(generation,iri),
  FOREIGN KEY(generation,justification) REFERENCES justification(generation,iri),
  CHECK(end IS NULL OR end>=start)
);
CREATE TABLE premise (
  generation INTEGER NOT NULL, justification TEXT NOT NULL,
  premise_claim TEXT NOT NULL, premise_occurrence TEXT NOT NULL,
  reviewed_revision TEXT NOT NULL,
  PRIMARY KEY(generation,justification,premise_occurrence),
  FOREIGN KEY(generation,justification) REFERENCES justification(generation,iri),
  FOREIGN KEY(generation,premise_claim) REFERENCES claim(generation,iri),
  FOREIGN KEY(generation,premise_occurrence) REFERENCES known_interval(generation,iri)
);
CREATE TABLE valid_interval (
  generation INTEGER NOT NULL, justification TEXT NOT NULL,
  known_occurrence TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('unknown','bounded')),
  start TEXT, end TEXT, bounds_justification TEXT,
  PRIMARY KEY(generation,justification,known_occurrence),
  FOREIGN KEY(generation,justification) REFERENCES justification(generation,iri),
  FOREIGN KEY(generation,known_occurrence) REFERENCES known_interval(generation,iri),
  FOREIGN KEY(generation,bounds_justification) REFERENCES justification(generation,iri),
  CHECK((kind='unknown' AND start IS NULL AND end IS NULL)
     OR (kind='bounded' AND (start IS NOT NULL OR end IS NOT NULL)
         AND bounds_justification IS NOT NULL)),
  CHECK(start IS NULL OR end IS NULL OR end>start)
);
CREATE TABLE claim_link (
  generation INTEGER NOT NULL, iri TEXT NOT NULL, from_claim TEXT NOT NULL,
  to_claim TEXT NOT NULL, kind TEXT NOT NULL
    CHECK(kind IN ('conflicts_with','supersedes')),
  from_justification TEXT NOT NULL, to_justification TEXT NOT NULL,
  at_known TEXT NOT NULL, policy_hash TEXT NOT NULL,
  PRIMARY KEY(generation,iri),
  FOREIGN KEY(generation,from_claim) REFERENCES claim(generation,iri),
  FOREIGN KEY(generation,to_claim) REFERENCES claim(generation,iri),
  FOREIGN KEY(generation,from_justification) REFERENCES justification(generation,iri),
  FOREIGN KEY(generation,to_justification) REFERENCES justification(generation,iri)
);
CREATE TABLE alias (
  generation INTEGER NOT NULL, entity TEXT NOT NULL, value TEXT NOT NULL,
  kind TEXT NOT NULL, claim TEXT NOT NULL, justification TEXT NOT NULL,
  PRIMARY KEY(generation,entity,value,kind,justification),
  FOREIGN KEY(generation,entity) REFERENCES entity(generation,iri),
  FOREIGN KEY(generation,claim) REFERENCES claim(generation,iri),
  FOREIGN KEY(generation,justification) REFERENCES justification(generation,iri)
);
CREATE TABLE merge (
  generation INTEGER NOT NULL, claim TEXT NOT NULL, left_entity TEXT NOT NULL,
  right_entity TEXT NOT NULL, CHECK(left_entity<right_entity),
  PRIMARY KEY(generation,claim),
  FOREIGN KEY(generation,claim) REFERENCES claim(generation,iri),
  FOREIGN KEY(generation,left_entity) REFERENCES entity(generation,iri),
  FOREIGN KEY(generation,right_entity) REFERENCES entity(generation,iri)
);
CREATE TABLE extraction_checkpoint (
  generation INTEGER REFERENCES generation(id), binding TEXT NOT NULL,
  extractor TEXT NOT NULL, version TEXT NOT NULL, vocabulary_hash TEXT NOT NULL,
  source TEXT NOT NULL, scope TEXT NOT NULL, last_capture TEXT NOT NULL,
  last_evidence_id INTEGER NOT NULL,
  PRIMARY KEY(generation,binding,extractor,version,vocabulary_hash,source,scope)
);
CREATE INDEX outgoing ON claim(generation,subject,predicate,iri);
CREATE INDEX incoming ON claim(generation,object_entity,predicate,iri)
  WHERE object_entity IS NOT NULL;
CREATE INDEX by_claim ON justification(generation,claim,iri);
CREATE INDEX by_evidence ON support_span(evidence,generation,justification);
CREATE INDEX by_premise ON premise(generation,premise_claim,justification);
CREATE INDEX by_alias ON alias(value,kind,generation,entity);
CREATE INDEX by_interval ON known_interval(generation,justification,start,end);
CREATE INDEX by_scope ON evidence_ref(scope,iri);
```

**Core invariants beyond DDL:** IRI syntax, datatype lexical/language consistency; `object_term` resolves an installed vocabulary IRI (e.g. the object of a supported `rdf:type` Claim), never an arbitrary undeclared resource; generation completeness; predicate/version registry; non-empty provenance spans; inference premises non-empty/acyclic; confirmation payload matching; exact retained UTF-8 span equality; source/scope/run grants; type claims; merge same-kind endpoints and symmetric hash; link endpoint/justification consistency; artifact input visibility; interval/status roll-up. JSON fields are bounded canonical manifests, never graph adjacency or executable rules. FK/SHACL checks cannot replace these checks. Runtime run IDs are audit metadata, excluded from stable support/revision hashing.

- Canonical identifiers generate lookup aliases even without an alias Claim, **only when the Entity has a visible justification path**. Login aliases are supported alias Claims; old aliases remain historical lookup candidates with intervals, not forever-current unique identifiers. Reused login lookup is ambiguous and never merges. This refines plan S13/ADR 0009 alias representation, not D7's exact-canonical lookup.
- Scope of a provenance justification = required Evidence scopes; inference = union of premise scopes; confirmation = its durable event scope. Availability requires **all** required scopes, not any one. Metadata tables do not confer visibility themselves.
- Support intervals/links stored for the full view are acceleration hints. Scoped state must reconstruct transitions from visible observations and policy; simply filtering globally closed intervals leaks hidden replacements. Use the same roll-up function on the scoped input subset, with stored adjacency/checkpoints—not fresh extraction.
- Materialized full-scope Claim Status/revision/Claim-presence intervals may be added as caches; scoped answers recompute them. Confirmation and retract events interleave deterministically by `(recorded_at, event IRI)`; Evidence order remains `(captured_at, Evidence ID)`.

**RDF 1.2 export fragment** (prefixes illustrative; all IDs minted IRIs):

```turtle
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix rc: <urn:rastro:core:> .
@prefix code: <urn:rastro:vocab:code:> .
@prefix ex: <urn:rastro:example:> .
ex:closed rdf:reifies <<( ex:pr12 code:state "closed" )>> .
ex:revision1 a prov:Entity ; rc:claim ex:closed ; rc:status rc:observed ;
  prov:wasGeneratedBy ex:run1 ; prov:qualifiedDerivation ex:support1 .
ex:support1 a prov:Derivation ; prov:entity ex:evidence1 ;
  rc:field "/state" ; rc:byteStart 0 ; rc:byteEnd 6 ; rc:quote "closed" ;
  rc:ruleVersion "github.state/1" ; rc:validTime rc:unknown .
ex:run1 a prov:Activity ; prov:wasAssociatedWith ex:extractor1 .
ex:extractor1 a prov:SoftwareAgent .
code:state skos:definition "The state reported by the source for an item." ;
  skos:scopeNote "Not proof that a commitment was fulfilled." ;
  skos:example "A merged GitHub PR has state closed." .
```

RDF 1.1 profile encodes the proposition as a named `rdf:Statement` with `rdf:subject/predicate/object`; reification alone likewise does not assert its triple. RDF 1.2 full/basic conversion must use the published interop mapping if claiming that standard's round trip, not label this custom RDF 1.1 profile “1.2-basic”. SHACL-style validation targets those explicit fields; `sh:class` requires a deliberately constructed, supported type view. No unqualified type inference from RDFS domain/range.

Standards checked 2026-11-05: [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/) remains CR Snapshot (2026-04-07); [PROV-O](https://www.w3.org/TR/prov-o/) and [SHACL 1.0](https://www.w3.org/TR/shacl/) are Recommendations. RDF 1.2 serialization/query/SHACL 1.2 readiness is not assumed; `10-models.md §1. RDF (triples, quads, RDF 1.2)` records draft statuses. No standards-derived retraction or authority algorithm is claimed.

## 4. Worked examples

### (a) PR #12: open → closed → open, two sources

PR Entity = D7 normalized URL; O/C = stable hashes of its open/closed propositions. A/B observe the same field/clock domain. Both visible here; source update clocks select current support, capture clocks define known intervals.

| Capture UTC, 2026-11-04 | Evidence; `/state` span | Projection after acceptance |
|---|---|---|
| 09:00 | A1 open `[0,4)`, upstream 08:55 | O observed; JA1 interval opens. |
| 09:02 | B1 open `[0,4)`, upstream 08:55 | O, two justifications; revision changes. |
| 10:05 | A2 closed `[0,6)`, upstream 10:00 | C observed; O superseded. JA1/JB1 close at 10:05; transition C→O records A2 replacing both. |
| 10:10 | B2 closed `[0,6)`, upstream 10:00 | C, two current justifications; revision changes. |
| 11:05 | B3 open `[0,4)`, upstream 11:00 | O observed again; C superseded; transition O→C. New O occurrence, same O ID. |
| 11:10 | A3 open `[0,4)`, upstream 11:00 | O two current justifications; another revision. |

- Ingest appends A1/A2/A3 and B1/B2/B3 separately: ADR 0005's A→B→A is three observations per source. Exact span verification precedes provenance acceptance; all these rules deterministic, Evidence structured.
- Presence O = `[09:00,10:05) ∪ [11:05,∞)`; C = `[10:05,11:05)`. Valid time unknown. These are full-view intervals; personal-only replay in (d) differs.
- Current `why(C)` reports superseded_by O; old C→O transition remains historical, not a concurrent permanent inverse relation. `why(O)` shows both presence intervals and bounded support history.
- Equal-clock disagreement preserves observed O and C plus conflict; absent/null field closes neither. As-of-known 10:07 reconstructs C with A2 support, not the B2 support learned later. Rebuild publishes identical IDs, intervals and visible revision fingerprints.

### (b) GitHub + Slack account → one Person → undo

“Merge accounts into a Person” is the owner's unified-person outcome; **literal account equality is rejected** (A4).

1. Structured GitHub Evidence identifies G=`github:account/42`; Slack Evidence identifies S=`slack:team/T1/user/U7`. Names are only aliases. G and S remain distinct account Entities.
2. Owner answers an Ask that both are held by the same human. Durable event K stores scope `personal`, reviewed evidence/claims and Person minting seed. Create P=`urn:rastro:person:<hash(K,person)>`; confirm `G heldBy P` and `S heldBy P`, each with its own target/revision and provenance trail. UI presents one Person with two accounts; G/S claims keep their subjects and IDs.
3. If two pre-existing Persons PG/PS existed, confirm symmetric Merge M=`merge(PG,PS)` instead. Their scoped resolved view groups duplicates while preserving member Claim IDs. Accounts retain original heldBy links. This is the actual glossary Merge operation.
4. Owner undo of the second holder link records a retract rule for `S heldBy P`; P/G remain, S returns to unresolved holder. In the duplicate-Person variant, undo records `block_merge(M)`; recompute components without M. Rebuild cannot silently recreate the blocked association/Merge.
5. Any inference depending on that association loses this justification. Another independent holder proof may survive **withdraw-confirmation**, but explicit retract blocks the targeted proposition. Pending Proposals withdraw if unsupported; Action runs using the old identity are flagged with proposed undo. No Claim subject is rewritten.
6. Work-only holder/Merge justifications are absent under personal, including member counts, representative selection and aliases. Owner confirmation defaults to the Ask's scope; no cross-scope propagation without another explicit intent.

### (c) Slack promise → confirmation → retraction carries through

Future hypothesis path; **outside the paused observed-only acceptance contract**.

1. Retain Evidence E, `/text`, bytes `[0,35)`: `I'll send the deck to Ana by Friday`. Preserve author Evidence separately. Core checks exact bytes (task `[5,18)`, mention `[22,25)`, due expression `[26,35)`). Artifact X retains ordered input context, vocabulary/prompt/model/settings and all outputs. No code changes or remote calls implied here.
2. Fixed standard vocabulary proposes Commitment K from `(message canonical ID, span, extraction rule semantic version)`; proposed claims include speaker `owes K`, `K owed_to Ana`, description `send the deck`, due expression `by Friday`, and supported type claims. Partial output remains in X until “I”/Ana resolve through supported Person links or an Ask; the name alone does not choose a Person. Relative Friday is not a calendar timestamp.
3. Once resolved, accept the complete bundle as hypotheses. Every interpretation has exact E support plus required author/identity premises. SHACL-style validation proves structure; it does not prove intent/entailment. No Act is authorized; untrusted-content-trigger cap in levels §9 still applies.
4. Owner confirms the displayed bundle: durable positive confirmation entries store each exact tuple/revision and the bundle hash. Status becomes confirmed; earlier hypothesis occurrences stay in history. A deterministic rule derives a reminder-basis Claim from the confirmed promise and independently justified date/identity claims. An Action still needs its own Level/rule checks; confirmation alone grants no permission.
5. Owner retracts the promise bundle: scoped retract rules target its exact claims, including the reviewed promise interpretation. Confirmation no longer sustains them; re-extraction of E cannot resurrect them. Reminder inference invalidates because a required premise is blocked. A conclusion with an unrelated complete justification survives; a separate date claim need not be retracted.
6. Unsupported pending reminder Proposal withdraws. Any previous Action run based on the promise is flagged and an undo proposed; sent messages are not erased. No reply is completion Evidence. Withdrawal of only the Owner's yes would instead return still-supported interpretations to hypothesis; this distinction is explicit in the intent UI.

### (d) `about(<account>)` and `why(<claim>)` under personal

Assume A in (a) is `work`; B is `personal`. Additional personal claim H=`PR12 code:author G` has structured `/user/id` Evidence `[0,2)` quote `42`; type and canonical URL claims have their own supported provenance. A work-only alias of G and work-only Person link also exist.

1. MCP launch fixes `--scope personal`; canonical `github:account/42` resolves G only through visible H/type support. `about(G)` performs incoming/outgoing scans: includes H and visible claims **directly incident on G**. It does not return PR12's state through H: that would be two hops. A visible personal heldBy link is one hop; it does not import the Person's other claims.
2. Compute all metadata from B/personal supports; omit A, work alias, work holder, hidden merge-member counts and work-only extraction state. Sort by newest visible Claim interval start then Claim ID; default 20/max 50, bounded counts and explicit truncation. Coverage lists visible pairs even with no returned claims.
3. Following H's PR12 endpoint with `about(PR12)` is the second semantic hop and returns O/C according to personal history. Optional future `expand` may bundle this traversal, returning ordered `[H,O]` Claim paths, not an inferred conclusion that authorship explains state.
4. `why(O)` personal: O intervals `[09:02,10:10) ∪ [11:05,∞)`, since B1 is replaced by B2 only at 10:10; A2 at 10:05 must not close B1. Current support B3 only; full scope O has A3 too. Scoped revision therefore differs; work-only transitions/counts/aliases are absent.
5. `why(H)` returns its own `/user/id` evidence, rule/vocabulary versions, extractor and span/excerpt; do not cite `/state` as author proof. UTF-8-safe excerpts capped at 400 bytes; full quotes only with `full`.
6. A hidden-only account yields the same empty `about` as unknown; hidden-only Claim yields the same `why` error as unknown. A hidden bridge never connects two personal endpoints. All reads pin one generation/transaction, perform no extraction or repair, and state Coverage, extraction lag/failure and retrieval completeness separately.

## 5. T4/T5 and migration

Branch facts are the inspected research snapshot, **not a new build verification**: `00-current-state.md §3. Unmerged branches (both from 1fb2108)`; plan §Pause.

| Work | Keep | Rework / drop |
|---|---|---|
| T4 `feat/claim-lifecycle` @4ac017f | Content IDs; lifecycle/recurrence/conflict fixtures; writer lock; failed-rebuild rollback; one-transaction activation concept; unknown valid time. | Replace global `refresh` on every insert with affected-slot recomputation plus full-replay oracle. Use occurrence transitions, explicit generation keys/manifest/checkpoints, scoped roll-up and typed Justifications. Drop hard-coded `Source::github` in rebuild. Do not strip FK constraints from staging. |
| T5 `feat/github-extractor` @7a532bb | Separate package/manifest; state/author/repository URL rules; numeric account identity; bot preservation; structured author spans; definition/evidence-requirement fields. | Type as supported RDF-type-equivalent Claim, not in-place mutable `entity.type`. Alias claims drive intervals/lookup, not untracked inserts. Explicit cardinality for state and meta-predicates. Validate against declared dependency registry, not only own namespace. |
| Location context | Need retained companion fields when only part of a record changed. | Specify evidence high-water cutoff and same location/source/scope; record exactly delivered ordered context and never fetch it during a read. Amends ADR 0008 §Shape/§Core validation; semantic context is not automatically current support. Bound context separately and retain input IDs for replay. |
| Combined interfaces | Reuse wire envelope and validation boundary. | Research reports textually clean merge **does not compile**: T4 references `predicate` outside T5's moved scope; type/alias have no defined cardinality. Resolve shared accepted-claim/vocabulary interfaces first. T5's absent state cardinality defaults to many and disables lifecycle replacement. Fix before claiming either acceptance result. |
| Tests | Preserve S1–S30 observable contracts where unchanged. | Rework S13 for historical aliases; add type lifecycle, old-version retirement, scoped hidden replacement, AND/OR retraction, merge undo and artifacts only when those features are authorized. T5's seven untagged tests still need plan tagging/converge; neither branch had code review in the snapshot. |

Migration proposal:

1. Preserve all existing immutable Evidence, run/Coverage data and migrations. Add a new timestamp migration; never edit applied migrations. Existing live DB had no Claim projection (`00-current-state.md §4. Real data`); do not assume every installation is equally empty.
2. Preserve legacy hash golden vectors. Add an injective legacy-ID↔IRI adapter; preserve old predicate hash keys. New terms/hash encodings get explicit versions. Export normalization registry, vocabulary packages and deterministic rule versions for replay elsewhere.
3. Re-extract retained structured Evidence into a new generation, compare semantic fixtures, atomically activate. Account author facts require the already agreed `--reread`; no replay can recover fields never captured. No stochastic path is enabled by this migration.
4. Later approved input tables for Owner intents/confirmations and artifacts must exist **before** projection relies on them. Export/forget/restore checks precede non-disposable use. Keep durable Owner-supplied propositions even if extractor/vocabulary changes reject their old derivation; report unmapped old terms rather than silently translating confirmations.
5. Reverting feature execution unbinds extractors and preserves precious inputs. Projection generations are disposable; stale exports/artifacts and retired generations must be purged on forgetting. Schema downgrade follows ADR 0002's unknown-migration refusal, not editing migration history.

Proposed ADR work, numbers allocated only after Owner approval:

- **Supersede ADR 0009:** identified reifiers/Justifications, occurrence transitions, generation publication, scoped revision v2 and identity overlays; keep its deterministic IDs, ordering and atomicity guarantees explicitly.
- **Amend ADR 0008:** supported type/alias Claims, versioned dependency registry, bounded location context, interpretation slots and explicit multiple bindings. Separate later hypothesis acceptance/artifact ADR; do not weaken observed checks.
- **New ADR:** reversible Person merges/account links, persistent blocking rules, duplicate grouping and intent targeting.
- **New ADR:** stochastic extraction artifacts, recorded replay vs re-extraction, retention/forget/export; dependent knowledge §12/sharing §1 amendment requires explicit Owner agreement.
- Keep ADR 0001–0007 architectures; amend ADR 0007 only for approved additive operation/result changes. ADR 0002 remains SQLite; no new engine ADR needed unless benchmarks justify one. No proposed concept edit; the explicit knowledge §5/§12 changes above must be approved first.

## 6. Risks and how we'd know we're wrong

Targets below are **proposed rejection gates**, not measured results or agreed budgets.

| Major choice / risk | Falsifiable test / reject or revisit when |
|---|---|
| RDF semantics over SQLite diverge | Round-trip all fixtures through full reifier and RDF 1.1 profiles. Compare exact propositions, support spans, scopes, occurrence links and lexical types. Any silent assertion of a hypothesis/retracted Claim, lost support or changed ID rejects the adapter. |
| Draft dependency | Pin RDF 1.2 profile; run W3C reification fixtures against a pinned parser independently. If syntax/semantics cannot round-trip without ambiguity, ship the RDF 1.1 profile only; no DB migration should be needed. Parser compatibility remains unverified here. |
| PROV/nanopub complexity pays no rent | Import a Share into a separate Owner DB; reconstruct sender attribution, exact supports and every revision without granting confirmed/Act status. Failure rejects package design. If no useful consumer needs it, defer serializer implementation. |
| SQL/scoped joins too slow | 100k/1M/10M Claims, sparse and 10k-degree hubs; cold/warm readers beside ingestion/rebuild. Proposed p95 about/why <200 ms warm at 1M, bounded expansion <500 ms; measure visited candidates, RAM, disk/WAL, rebuild time and peak disk. Misses on fixed hardware after index/query tuning justify native RDF comparison, not guessed engine superiority. |
| Scope roll-up leaks | Metamorphic test: add/remove arbitrary hidden evidence, replacements, merges, confirmation/artifacts. Personal graph answers/revisions unchanged (except documented FTS rank-statistics limit). Any hidden bridge/status/alias/count change is a correctness failure. |
| Recurrence/time ambiguous | A→B→A, equal clocks, stale mirrors, late captures, missing fields; incremental output equals full scoped replay at each cutoff. Presence intervals and occurrence transitions must match (a)/(d); any invented valid bound fails. |
| Generation publication unsafe | Crash before/after artifact recording, checkpoint commit, rebuild validation and active-pointer commit while separate readers loop. Every answer internally old or new; retry idempotent; no lost accepted input. Any mixed generation/coverage snapshot or lost checkpoint work fails. |
| Identity overlay violates one identity | Merge/unmerge chain incl. shared account, negative rule and hidden member; original renderer targets resolve throughout. UI has one grouped Person view without falsely merging accounts. Any re-key, silently recurring blocked merge or false transitive cut fails; unusable grouped targeting warrants revisiting knowledge §5 amendment. |
| SHACL profile over/under-validates | Differential fixtures against SHACL 1.0-compatible exported shapes; malformed domains/datatypes rejected; contextual scalar conflicts accepted/reported. Reject if profile accepts malformed input or suppresses contradictions. Full semantic entailment deliberately not a passing criterion. |
| LLM grounding insufficient | Owner-labelled promise/negation/quotation/joke/identity/date fixtures; compare vocabulary prompt vs constrained output at equal model/token budget. Zero invalid spans/automatic confirmations; propose ≥95% hypothesis precision before surfacing without review, report recall separately. If vocabulary eliminates useful relations, revise definitions/adapter—not just precision threshold. |
| Durable artifacts fail replay | Disable model/network, wipe projection/cache, rebuild from exported precious inputs on another machine: identical IDs, accepted submissions and scoped revisions. Any required provider call or lost Owner yes fails. Measure artifact growth per 1k messages; Owner decides acceptable storage. |
| Retraction drops alternatives or leaves cycles | J1=A∧B, J2=C: retract A, conclusion survives via C; retract C, conclusion loses last proof. Attempt cycles must reject. Explicit target block defeats all proofs; undo-confirmation does not. Pending Proposals and Action runs reflect basis loss without pretending external undo occurred. |
| Several extractors inflate truth | Same Evidence through two rules/versions; no independence bonus, failed upgrade retains old good output, completed slot replacement retires old interpretation. Any mixed-version drift from rebuild fails. |
| Expansion adds noise | Held-out 40–60 Owner questions; FTS vs one-hop/two-hop at equal context tokens and latency, including bot-as-noise/bot-as-answer. Require improved sufficient-evidence recall without scope/citation regressions; otherwise retain fixed APIs and FTS. |
| Forgetting incomplete | Forget Evidence behind a merge/confirmed hypothesis, rebuild/restore all still-retained generations and artifacts; forgotten content cannot reappear in queries, excerpts or export. Any surviving copy/reingestion without honoring marker fails. |

Benchmark methods: `12-engines.md §Personal-scale fit: 10k–1M evidence rows, 100k–10M claims`; `14-retrieval.md §Evaluation for the owner`; artifact/grounding caveats: `15-llm-ontology.md §Unverified`. No engine, performance or extraction benchmark was run for this proposal.

## 7. Questions for the Owner — ranked

| Rank | Decision / options | Leaning and approval consequence |
|---|---|---|
| Q1 | **Representation + engine:** (a) RDF profile/IRIs over SQLite; (b) Oxigraph owned by one service; (c) immutable Oxigraph generations beside SQLite. | **a**: preserves independent readers and library-first embedding. b amends ADR 0006/process model; c needs cross-store snapshot/forget protocol. |
| Q2 | **Identity:** (a) Person hub + supported account links, merge overlay and grouped immutable Claim IDs; (b) re-key Claims on merge with redirects; (c) defer Person/merge execution. | **a**, explicit knowledge §5 clarification/amendment. Not literal account equality. Confirm default scope = Ask context; cross-scope confirmation requires explicit choice. |
| Q3 | **LLM replay:** (a) artifacts become precious inputs and portable export; (b) only confirmed interpretations retained, unconfirmed hypotheses may change on rebuild; (c) best-effort model replay. | **a**, amends knowledge §12/sharing §1. b/c must weaken exact replay guarantees explicitly; c cannot satisfy identical stochastic rebuild. |
| Q4 | **Owner correction:** (a) yes independently sustains the exact reviewed proposition; retract blocks it until reversed, undo-yes only removes confirmation; (b) yes expires whenever provenance changes; (c) confirmation bound to one occurrence with explicit renewal. | **a** for deliberate corrections; more retention and scoped event policy. Never transfer yes to a changed proposition; forgetting still carries through. |
| Q5 | **Agent queries:** (a) fixed APIs + vocabulary references, then evaluated bounded expansion; (b) fixed APIs only; (c) restricted SPARQL now. | **a**; keep `about` one hop/D15 ordering. New expansion needs catalog approval and explicit work/output budgets. |
| Q6 | **Time:** (a) capture-cutoff reconstruction with pinned today's interpretation; (b) additionally retain publication/answer receipts for exact served history; (c) contextual Claim IDs including temporal qualifiers. | **a** initially; b adds precious audit history policy. c revises D5; future propositions whose meaning includes a date may need explicit contextual Entities. |
| Q7 | **Validation:** (a) bounded SHACL-style profile compiled from installed vocabularies, SHACL 1.0-compatible export; (b) full SHACL engine; (c) bespoke schema without standard mapping. | **a**, with differential fixtures; cardinality conflicts always report-only. No automatic OWL entailment. |
| Q8 | **Interop timing:** (a) publish semantic/export profile now, build serializer when sharing/export work starts; (b) ship nanopublication-style serialization immediately; (c) keep only internal JSON. | **a**; b increases present scope, c forfeits an independently inspectable standard representation. |
| Q9 | **Extractor composition:** (a) explicit lists + interpretation-slot version retirement now; (b) define slots now, keep one binding until a second extractor needs them; (c) keep old-version supports current indefinitely. | **b** for present scope, adopting A8's model when list execution is approved. c accumulates obsolete interpretations. |
| Q10 | **Paused work:** (a) resume T4/T5 after agreeing observed-only seams and revised ADR 0009; (b) replace paused plan with the complete approved model; (c) merge existing branches unchanged. | **a** keeps useful work without implementing future hypotheses/merges prematurely. c rejected on compile and semantic evidence; Owner chooses a/b and the exact approved changes. |

## Unverified / boundaries

- DDL creation checked, not a production migration, complete validator or implementation benchmark. RDF fragment documents mapping; no installed RDF parser/conformance run here.
- SHACL compiler equivalence, full RDF 1.2 engine feature coverage, serializer ecosystem compatibility and proposed latency/precision thresholds remain unmeasured.
- T4/T5 code facts/revisions come from `00-current-state.md`; their gates were not rerun. No branch, git state, live database, decisions, ADRs or other research files changed.
