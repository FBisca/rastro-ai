# 10: Graph data models for a cited, time-aware personal graph

Author: `kg-models` (step 1 research). Checked 2026-11-04. Covers areas A1, A2, A5 and A6, plus interop for sharing. Nothing here is decided. Builds on `docs/research/knowledge-graphs-2026-11.md` without repeating it.

## Bottom line
- Every viable model ends up with a **claim node**: an identified thing that a status, a revision, supports, intervals and claim-to-claim links can hang off. The models differ in what that node is called and what the standard gives you around it:
  - RDF 1.2: a reifier.
  - LPG: a node, because an edge can't be the subject of another edge.
  - Wikibase: a statement.
  - TypeDB: a relation.
  - Datalog/EAV: an entity id.
  - Relational: a row (`docs/adr/0009-claims-projection.md:13`).
- ADR 0009's tables already are the **statement-node (n-ary) pattern** in relational form. Choosing a "graph model" mostly means choosing:
  1. which standard vocabulary and semantics to align ids and exports with;
  2. whether rules such as status roll-up and supersession are written as code or declaratively;
  3. which query language agents and views get.
- **Standards status (2026-11):**
  - RDF 1.2 Concepts is a Candidate Recommendation (CR) Snapshot (2026-04-07). RDF 1.2 Semantics is a CR Draft (2026-09-24). SPARQL 1.2, Turtle 1.2 and SHACL 1.2 are still Working Drafts (WDs), dated 2026-11-01 to 2026-11-02. JSON-LD has no triple terms until the JSON-LD WG delivers; first drafts are planned around Q4 2026.
  - On the LPG side, ISO GQL has been published since 2024-04-12, and SQL/PGQ is part 16 of SQL:2023. PostgreSQL 19 ships SQL/PGQ without variable-length paths.
- **Open-world reasoning (OWL) clashes with Rastro in three places:**
  - Rastro keeps contradictions (`docs/decisions/vocabulary.md:36`), but a functional property with two different literal values makes an OWL ontology inconsistent.
  - `owl:sameAs` can't be undone (`docs/decisions/knowledge.md:11`).
  - Absence has to be explicit coverage (§11), not something left to inference.
  - SHACL-style closed-world validation that reports violations fits; OWL entailment doesn't.
- Sharing exports **claims plus justifications as evidence from a person** (`docs/decisions/sharing.md:8`). Export proper is evidence, confirmations, rules and outcomes (`docs/decisions/sharing.md:6`), not claims. So the internal model and the wire format can be chosen separately, and the interop cost sits in one serializer.

## The running example
PR `https://github.com/acme/app/pull/12` (entity `e7f3a`, content hash per ADR 0009):
- Claim `c41aa`: `code:state "open"`, one support `s_z9` (evidence `055`, source `github-work`, scope `work`, `/state` bytes [0,4) "open", rule `github.state/1`, captured 2026-10-01T09:00Z). Status `superseded`. Known interval [2026-10-01T09:00Z, 2026-11-03T10:00Z).
- Claim `c9b21`: `code:state "closed"`. Two current supports:
  - `s_a1`: evidence `101`, source `github-work`, scope `work`, captured 2026-11-03T10:00Z;
  - `s_b2`: evidence `202`, source `github-oss`, scope `personal`, captured 2026-11-03T12:30Z.

  Both cover `/state` bytes [0,6) "closed", rule `github.state/1`. Status `observed`, revision `5d0e`, known interval [2026-11-03T10:00Z, open), valid time `unknown`, `supersedes c41aa`.

## 1. RDF (triples, quads, RDF 1.2)
**Core facts**
- **Triples and quads.** A graph is a set of (s, p, o) triples, and a dataset adds named graphs, which makes them quads. Named graphs have no agreed semantics: what a graph name denotes is up to the application.
- **Blank nodes** are local, existential and unstable across serializations. RDF Dataset Canonicalization (RDFC-1.0, a W3C Recommendation) exists to hash and diff graphs that contain them. For content-addressed ids, mint IRIs instead.
- **RDF 1.2 triple terms:**
  - A triple can be the object of another triple, written `<<( s p o )>>`, and *only in the object position*.
  - A triple term is not asserted. It is linked by a **reifying triple** `r rdf:reifies <<( s p o )>>`, and the reifier `r` carries the statements about it.
  - Turtle 1.2 sugar: `<< s p o ~ r >>` reifies the triple without asserting it. `s p o ~ r {| … |}` asserts it *and* annotates it.
  - One triple can have several reifiers, for example one per claim or per source.
  - Conformance comes in two levels, "full" and "basic" (version labels `1.2` and `1.2-basic`). The RDF 1.2 Interoperability note gives a basic encoding (a blank node of type `rdf:PropositionForm`, about four triples per triple term) that round-trips to full.
- **SPARQL 1.2** (WD 2026-11-01): `<<( )>>`, `<< ~ ?r >>` and `{| |}` patterns, plus `TRIPLE`, `SUBJECT`, `PREDICATE`, `OBJECT` and `isTRIPLE`, and `VERSION "1.2"`.
- **SHACL 1.2** (Core WD 2026-11-02) is closed-world validation that *reports*, with severities `sh:Violation`, `sh:Warning` and `sh:Info`, and reifiers can carry a per-constraint severity. Rules moved to a separate WD, SHACL 1.2 Inference Rules (2026-10-31), together with "SPARQL 1.2 RL", a Datalog-style rule language. Node Expressions, Profiling, UI and Compact Syntax are separate drafts.
- **OWL 2** (Recommendation, 2012): open world and no unique-name assumption.
  - `rdfs:domain` and `rdfs:range` *infer* types; they don't reject anything.
  - A functional property with two values merges two individuals or, for literals, makes the ontology inconsistent.
  - `owl:sameAs` is symmetric and transitive, and nothing can undo it.
- **JSON-LD 1.1** (Recommendation, 2020-07-16) has no triple terms. The JSON-LD WG charter (approved January 2026) adds RDF 1.2 triple terms and directional strings, with first public drafts planned around Q4 2026 (the draft charters disagree on dates).

**Weak spots for Rastro**

| Need | Problem in RDF |
|---|---|
| n-ary claim (claim + status + supports + intervals) | Needs a reifier, a statement node or a named graph per claim. Plain triples can't carry it. The 2006 W3C note on N-ary relations is still the guidance |
| Provenance | No built-in. PROV-O (Recommendation, 2013) or a custom vocabulary on the reifier. Nanopublications put each assertion in its own named graph with provenance and pubinfo graphs. A 2025 paper adds a fourth graph because one provenance graph "can only represent the fact as originating from a single source", which is exactly Rastro's "≥1 supports" |
| Identity | IRIs are global (good for sharing). `owl:sameAs` is the standard merge, and it is irreversible under reasoning, so merges have to be reified claims (e.g. `rc:sameAs` on a reifier), never `owl:sameAs` in the store |
| Literals as subjects | Not allowed. Fine, Rastro claims have entity subjects |
| Unknown valid time | Open world means a missing triple is "unknown" by default. That's convenient, but `unknown` should still be explicit so it isn't mistaken for "not yet extracted" |
| Lists (per-index evidence, R38) | `rdf:List` is awkward to query; use indexed nodes |

**Example (Turtle 1.2)**
```turtle
VERSION "1.2"
PREFIX rc:   <https://rastro.dev/ns/core#>
PREFIX code: <https://rastro.dev/ns/code#>
PREFIX xsd:  <http://www.w3.org/2001/XMLSchema#>
PREFIX e: <urn:rastro:entity:>  PREFIX c: <urn:rastro:claim:>
PREFIX s: <urn:rastro:support:> PREFIX ev: <urn:rastro:evidence:>

e:e7f3a a code:PullRequest ; rc:alias "https://github.com/acme/app/pull/12" .

# current claim: triple asserted, reifier = claim id
e:e7f3a code:state "closed" ~ c:c9b21 {|
  rc:status rc:observed ; rc:revision "5d0e" ; rc:validTime rc:unknown ;
  rc:knownInterval [ rc:start "2026-11-03T10:00:00Z"^^xsd:dateTime ] ;
  rc:support s:a1 , s:b2 ;
  rc:supersedes c:c41aa
|} .
s:a1 rc:evidence ev:101 ; rc:field "/state" ; rc:byteStart 0 ; rc:byteEnd 6 ;
     rc:quote "closed" ; rc:rule "github.state/1" ; rc:current true .
s:b2 rc:evidence ev:202 ; rc:field "/state" ; rc:byteStart 0 ; rc:byteEnd 6 ;
     rc:quote "closed" ; rc:rule "github.state/1" ; rc:current true .
ev:101 rc:source "github-work" ; rc:scope rc:work ;     rc:capturedAt "2026-11-03T10:00:00Z"^^xsd:dateTime .
ev:202 rc:source "github-oss"  ; rc:scope rc:personal ; rc:capturedAt "2026-11-03T12:30:00Z"^^xsd:dateTime .

# superseded claim: reified, NOT asserted
<< e:e7f3a code:state "open" ~ c:c41aa >>
  rc:status rc:superseded ; rc:support s:z9 ;
  rc:knownInterval [ rc:start "2026-10-01T09:00:00Z"^^xsd:dateTime ;
                     rc:end   "2026-11-03T10:00:00Z"^^xsd:dateTime ] .
```
- Asserted vs unasserted lines up with "current" vs history. Rastro would still have to pick a rule for `hypothesis` and `retracted`, probably "unasserted".
- Variants:
  - **Named graph per claim** (nanopub style): flexible, but it competes with using graphs for scope or source, and means one graph per claim.
  - **Wikidata statement nodes** (`p:` / `ps:` / `pq:` plus `prov:wasDerivedFrom`): RDF 1.1-compatible today.
  - **Classic `rdf:Statement`**: four triples per claim; legacy.

## 2. Labelled property graphs (LPG)
**Core facts**
- **Data model.** Nodes and edges carry labels and key/value properties. Edges are directed and binary. Property values are scalars or lists of scalars; Neo4j allows no nested maps.
- **ISO/IEC 39075 GQL** (published 2024-04-12):
  - Graph types can be *open* (anything goes; Neo4j's declared minimum is feature GG01) or *closed* (only declared node and edge types, properties and endpoint pairs).
  - Neo4j docs, updated 2026-04-30, say Cypher supports "the majority" of mandatory GQL features, with a few still missing. Microsoft Fabric graph documents GQL conformance, including closed graph types.
- **SQL/PGQ** (ISO/IEC 9075-16, SQL:2023): `CREATE PROPERTY GRAPH` as a *view over existing tables*, queried with `GRAPH_TABLE(… MATCH …)`.
  - Oracle ships it.
  - PostgreSQL 19 committed it on 2026-03-16 (GA unverified, see Unverified), core features only, with **no quantified (variable-length) paths**. Its planner rewrites a pattern into joins.
  - DuckDB has it through the DuckPGQ community extension.
  - SQLite has no SQL/PGQ (nothing found).
- **openCypher** is the de-facto dialect, converging on GQL. Its 2026 status as a separate spec wasn't found.
- **PG-Schema** (SIGMOD 2023) gives node and edge types with multiple inheritance, plus **PG-Keys** (SIGMOD 2021) for key, uniqueness and participation constraints. It is research that fed GQL, not a shipped standard. The paper found that none of 11 engines supported schemas comprehensively.

**Weak spots for Rastro**

| Need | Problem in LPG |
|---|---|
| Claim as subject of statements | An edge can't be the endpoint of another edge, so supports, `supersedes` and `conflicts_with` can't point at an edge-claim. Graphiti stores facts as `RELATES_TO` edges and keeps provenance as a **list of episode UUID strings** in a property: no referential integrity, no span per support |
| Literal objects | `code:state "closed"` needs either a value node or the value as a claim-node property |
| Global identifiers | None in the model. Internal ids are engine-local and unstable, so you need an `id` property plus a uniqueness constraint (vendor DDL) |
| Schema | GQL closed graph types cover types, properties and endpoints. Cardinality per subject, definitions, examples and "not to confuse with" are not standard. Constraint DDL is vendor-specific |
| Many known intervals per claim | Parallel list properties or interval nodes |
| Merges | Vendor helpers merge nodes destructively (e.g. APOC `mergeNodes`). A reversible merge has to be a claim node plus resolution at query time |

**Example (Cypher, claim as node; recommended shape if LPG)**
```cypher
MERGE (pr:Entity:`code:PullRequest` {id:'e7f3a'})
MERGE (pr)-[:HAS_ALIAS]->(:Alias {value:'https://github.com/acme/app/pull/12', kind:'url'})
MERGE (e1:Evidence {id:'101', source:'github-work', scope:'work',     captured_at:datetime('2026-11-03T10:00Z')})
MERGE (e2:Evidence {id:'202', source:'github-oss',  scope:'personal', captured_at:datetime('2026-11-03T12:30Z')})
CREATE (c2:Claim {id:'c9b21', predicate:'code:state', object_kind:'str', object:'closed',
                  status:'observed', revision:'5d0e', valid_time:'unknown'})-[:SUBJECT]->(pr)
CREATE (c2)-[:SUPPORTED_BY {field:'/state', byte_start:0, byte_end:6, quote:'closed',
                            rule:'github.state/1', current:true}]->(e1)
CREATE (c2)-[:SUPPORTED_BY {field:'/state', byte_start:0, byte_end:6, quote:'closed',
                            rule:'github.state/1', current:true}]->(e2)
CREATE (c2)-[:KNOWN]->(:Interval {start:datetime('2026-11-03T10:00Z')})
CREATE (c2)-[:SUPERSEDES]->(c1:Claim {id:'c41aa', predicate:'code:state', object:'open', status:'superseded'})
CREATE (c1)-[:KNOWN]->(:Interval {start:datetime('2026-10-01T09:00Z'), end:datetime('2026-11-03T10:00Z')})
// optional derived shortcut for traversal:
// (pr)-[:`code:state` {claim:'c9b21'}]->(:Value {v:'closed'})
```
- A support fits an edge with properties: it is binary (claim, evidence) plus attributes. But nothing can then point *at* a support.
- With claim-as-node, one logical hop (entity → entity) becomes two physical hops. For `about`, which is one hop (`docs/decisions/knowledge.md:29`), that's acceptable.

## 3. Others
**Statement graphs / n-ary (Wikibase)**
- A statement has a main snak (property plus value, `somevalue` or `novalue`), qualifiers, references (sets of snaks) and a rank (`preferred`, `normal`, `deprecated`). The RDF export gives each statement a node (`p:` → statement → `ps:` value; `pq:` qualifiers; `prov:wasDerivedFrom` references; `wikibase:rank`) plus a "truthy" `wdt:` triple.
- How it maps:
  - references ≈ supports;
  - qualifiers carry valid time by convention (P580/P582);
  - rank ≈ status, lossily: 3 values against 5;
  - **no known time** on the statement, only page revision history;
  - supersession by convention (the old value keeps an end-time qualifier, the new one gets preferred rank), not as a link.

**TypeDB 3** (Rust rewrite; "polymorphic" hypergraph-like model)
- Relations have roles, can have **any number of role players**, and can themselves play roles in other relations. Lists of role players arrived in 3.0.
- `@card(N..M)` applies to `owns`, `relates` and `plays`; `@key` gives identity.
- So claim, support and supersession are first-class relations, and the schema is strong.
- Costs:
  - The schema lives in the database: a vocabulary version is a schema migration, not just a replay.
  - It's a separate server (that's A7, not covered here).
  - TypeQL is its own language with no standard behind it.

**Datalog / EAV** (Datomic, XTDB, Nemo)
- Everything is a fact `[e a v tx op]`, and rules derive the rest.
- **Datomic:** binaries free under Apache 2.0 since 2023, not open source.
- **XTDB 2:** bitemporal SQL (SQL:2011 periods).
- **Nemo:** Rust Datalog with stratified negation and existential rules, usable as a crate.
- Strong fit with "knowledge = f(evidence, confirmations, rules)" (`docs/decisions/knowledge.md:26`): ADR 0009's roll-up and supersession rules become a stratified program.
- **Catch:** the database's transaction or system time is when the *engine* stored a fact. A rebuild resets it. Rastro's known time is evidence `captured_at` (`docs/decisions/knowledge.md:48`), so it has to stay a plain attribute.
- Provenance of *derived* facts (why-provenance) isn't automatic. The justification has to be written as facts.

**Relational claim tables + graph view** (the current ADR 0009)
- Everything is explicit columns and foreign keys. Nothing is free: domain, range and vocabulary checks live in the core (plan D2). A graph view comes from recursive CTEs (SQLite) or `GRAPH_TABLE` (PostgreSQL 19, DuckPGQ, Oracle).

**Examples: same claim in the other models**
```text
-- Relational (ADR 0009 rows)
entity        (e7f3a, code:PullRequest, https://github.com/acme/app/pull/12)
claim         (c41aa, e7f3a, code:state, str:"open",   superseded, r:91c2)
claim         (c9b21, e7f3a, code:state, str:"closed", observed,   r:5d0e)
claim_support (c9b21, ev101, /state, 0, 6, "closed", github.state/1, current=1)   -- via ev101: github-work/work
claim_support (c9b21, ev202, /state, 0, 6, "closed", github.state/1, current=1)   -- via ev202: github-oss/personal
claim_support (c41aa, ev055, /state, 0, 4, "open",   github.state/1, current=0)
claim_link    (c9b21 -> c41aa, supersedes)
known_interval(c41aa, 2026-10-01T09:00Z, 2026-11-03T10:00Z)
known_interval(c9b21, 2026-11-03T10:00Z, NULL)            -- valid time: unknown
```
```json
// Wikibase-style statement (on entity e7f3a)
{"id":"e7f3a$c9b21","rank":"preferred",
 "mainsnak":{"snaktype":"value","property":"code:state","datavalue":{"type":"string","value":"closed"}},
 "qualifiers":{},
 "references":[
   {"snaks":{"rc:evidence":["101"],"rc:span":["/state[0,6)"],"rc:rule":["github.state/1"]}},
   {"snaks":{"rc:evidence":["202"],"rc:span":["/state[0,6)"],"rc:rule":["github.state/1"]}}]}
// c41aa: same shape, value "open", rank "normal"; no known interval, no supersedes link (lossy)
```
```typeql
# TypeDB 3 (sketch, not run)
define
  entity pull-request, owns canonical-id @key, plays claim:subject;
  entity evidence, owns evidence-id @key, plays support:evidence;
  relation claim, relates subject @card(1), owns predicate, owns object-text, owns status, owns revision,
    plays support:claim, plays supersession:newer, plays supersession:older;
  relation support, relates claim, relates evidence,
    owns field, owns byte-start, owns byte-end, owns quote, owns rule, owns current;
  relation supersession, relates newer, relates older;
insert
  $c2 isa claim, links (subject: $pr), has predicate "code:state", has object-text "closed", has status "observed";
  (claim: $c2, evidence: $ev101) isa support, has field "/state", has byte-start 0, has byte-end 6, has rule "github.state/1";
  (claim: $c2, evidence: $ev202) isa support, has field "/state", has byte-start 0, has byte-end 6, has rule "github.state/1";
  (newer: $c2, older: $c1) isa supersession;
```
```prolog
% Datalog: base facts from the extractor, rules = ADR 0009 roll-up
claim(c9b21, e7f3a, "code:state", str("closed")).   claim(c41aa, e7f3a, "code:state", str("open")).
support(s_a1, c9b21, ev101, "/state", 0, 6, "github.state/1").
support(s_b2, c9b21, ev202, "/state", 0, 6, "github.state/1").
support(s_z9, c41aa, ev055, "/state", 0, 4, "github.state/1").
replaced_by(s_z9, ev101).            % newer comparable observation, same (source, scope, location, field, rule)
current(S)          :- support(S,_,_,_,_,_,_), not replaced_by(S,_).
live(C)             :- support(S,C,_,_,_,_,_), current(S).
status(C,observed)  :- claim(C,_,_,_), live(C).
status(C,superseded):- claim(C,_,_,_), not live(C).
supersedes(N,O)     :- replaced_by(S,E), support(S,O,_,_,_,_,_), support(_,N,E,_,_,_,_),
                       claim(O,X,P,_), claim(N,X,P,_), scalar(P).
```

## 4. Rastro's needs mapped to each model

| Need (source) | Relational (ADR 0009) | RDF 1.2 | LPG (GQL/Cypher) | Statement / n-ary (Wikibase, TypeDB) | Datalog / EAV |
|---|---|---|---|---|---|
| Claim id, content hash (ADR 0009 §Ids) | PK column | Reifier IRI `urn:rastro:claim:<hash>`, globally valid | `id` property + uniqueness constraint | Statement GUID / `@key` | Entity id |
| Status (5 values) | Column | Triple on reifier; asserted vs unasserted is an extra signal | Node property | Wikibase rank (3 values, lossy); TypeDB attribute | Derived rule |
| Revision (hash of visible state) | Column, recomputed per scope | Triple; recomputed per scope | Property; recomputed per scope | Attribute | Derived (needs hashing outside Datalog) |
| ≥1 support with evidence + span + rule | `claim_support` rows | Support nodes with IRIs off the reifier | `SUPPORTED_BY` edges with properties, or support nodes | Wikibase references (any number); TypeDB `support` relation | `support/7` facts |
| Known intervals, several per claim (A→B→A) | Rows | Interval nodes (OWL-Time is a CR Draft, optional) | Interval nodes | Not in Wikibase; TypeDB relation | Facts; *not* the engine's transaction time |
| Valid time, `unknown` allowed | Column / flag | Explicit `rc:validTime rc:unknown` (absence is ambiguous) | Property | Wikibase qualifiers P580/P582 + `somevalue` | Fact |
| `conflicts_with`, `supersedes` (claim→claim) | `claim_link` rows | Triples between reifiers | Edges between claim nodes (impossible if claims are edges) | Not first-class in Wikibase; TypeDB relation | Derived rules |
| Scopes, derived per caller (knowledge §14) | Filter supports, recompute | Graph per (source, scope) or a triple on evidence; derived fields still recomputed at query time | Property filter; recompute | Same | Rule parameterised by visible scopes |
| Aliases | `alias` rows | `rc:alias` triples (not `owl:sameAs`) | Alias nodes / property | Wikibase aliases (label-like only) | Facts |
| Merges as claims, reversible | Merge claim + union-find in projection | Reified `rc:sameAs` claim; export may *emit* `owl:sameAs` for current merges | Merge claim node; never `mergeNodes` | TypeDB relation; Wikibase redirects aren't reversible claims | Rule: `same(X,Y)` from live merge claims, recursive |
| Namespaced vocabularies | Vocabulary table + core checks | Native: IRIs are namespaces | Label prefixes by convention | Wikibase properties `P…`; TypeDB schema | Attribute namespaces (Datomic `:ns/attr`) |
| Domain / range | Core check (plan D2) | `rdfs:domain`/`range` *infer*; enforce with SHACL `sh:class`/`sh:datatype` | GQL closed graph type endpoints | Wikibase property constraints (reports); TypeDB `plays`/`relates` (enforced) | Integrity rules |
| Cardinality per subject and context, contradictions kept (vocabulary §13) | Core logic | SHACL `sh:maxCount` + `sh:severity sh:Warning`; OWL `FunctionalProperty` would be *wrong* | Not standard | Wikibase "single-value constraint" is report-only (good fit); TypeDB `@card` *rejects* (bad fit) | Rule that derives a conflict |
| Definition, not-to-confuse-with, examples (vocabulary §5) | Vocabulary file | `skos:definition`, `skos:scopeNote`, `skos:example` | None | Wikibase property descriptions | None |

## 5. Comparison across the decision areas

| | Relational claim tables | RDF 1.2 (+SPARQL/SHACL) | LPG (GQL / Cypher / SQL/PGQ) | Statement / n-ary (Wikibase, TypeDB) | Datalog / EAV |
|---|---|---|---|---|---|
| **A1 Data model** | Fits all needs; all structure is hand-designed; already built | Fits via reifiers; global ids; standards still WD/CR | Fits only with claim-as-node; edge-as-claim breaks A2 | Closest native shape for "statement + refs + qualifiers"; Wikibase lacks known time | Fits; claims as entities, logic as rules |
| **A2 Reification / provenance** | Explicit tables; FK integrity | Reifiers + PROV-O or custom; nanopub precedent; asserted/unasserted split | Support edges OK; claim-on-claim needs nodes; Graphiti's id-list is the anti-pattern | Wikibase references = supports; TypeDB relations of relations | Facts; derivation provenance must be modelled by hand |
| **A5 Vocabulary / schema** | Custom file + core checks | Strongest standard stack (RDFS, SKOS, SHACL 1.2 with severities); avoid OWL reasoning | Weakest standard (GQL graph types; PG-Schema is research) | TypeDB strong but enforcing; Wikibase constraints report-only | Weak standard; rules can express checks |
| **A6 Traversal / indexes** | B-tree on (subject, predicate), (object); recursive CTE; 1–2 hops easy | SPO/POS/OSP permutation indexes; property paths; supports cost one join via reifier | Native adjacency, variable-length paths (not in PG19 PGQ); claim-node doubles hops | TypeDB native; Wikibase via SPARQL | Recursion native; indexes engine-specific |
| **Interop to RDF** | Mapping (R2RML/RML or code) | Native | Mapping (n10s, OneGraph research) | Wikibase has an RDF dump format | Mapping |
| **Rebuildable projection** | Yes (built) | Yes | Yes | Yes | Yes, most naturally (the program *is* the projection) |

## 6. Interop costs

| Direction | Cost | Notes |
|---|---|---|
| Relational → RDF 1.2 | Low–medium | One mapping: claim → reifier, support → node. Content-hash ids become `urn:` IRIs as they are. R2RML (Recommendation, 2012) can't emit triple terms; RML-star is a Final Community Group report (2025-10-31), and an RML WG charter targets a Recommendation around Q2 2027. A hand-written Rust serializer is probably smaller than adopting RML |
| RDF 1.2 → relational | Low for a generic quad/EAV table, medium for typed tables | Blank nodes need skolemising; a reifier reifying several triple terms has no single-row equivalent |
| LPG → RDF 1.2 | Medium | Edge with properties → reifier (natural); labels → `rdf:type`; list properties lose order unless encoded. Neo4j `n10s` imports/exports RDF; its RDF-star or RDF 1.2 support isn't confirmed. Amazon's OneGraph ("1G") still frames full RDF↔LPG unification as a vision (Semantic Web journal, 2023; LDBC 2024) |
| RDF → LPG | Medium–high | Multi-valued properties, language tags, datatypes, blank nodes; an *unasserted* reified triple has no edge to hang off, so it must become a node |
| RDF 1.2 full → basic (RDF 1.1 tools) | Low | Interop note encoding, about 4 triples per triple term; round-trip formalised (Tomaszuk & Champin, SWJ 2026, under minor revision) |
| Any → JSON-LD | Low for the statement-node shape | JSON-LD 1.1 carries statement nodes today; triple terms wait for the JSON-LD WG (drafts around Q4 2026) |

**Sharing** (`docs/decisions/sharing.md:7-8`)
- A share is *claims + justification*, received as evidence from a person.
- The nearest existing format is a **nanopublication**: assertion, provenance and pubinfo graphs, with signatures and trusty URIs in that ecosystem.
- An RDF 1.2 equivalent is a reifier per claim plus its supports, in TriG or JSON-LD.
- Content-hash claim ids are global if the canonical encoding is versioned and published. Two owners deriving the same claim get the same id, which helps the receiver match a shared claim to its own. Revisions stay per owner.
- None of this needs RDF *inside* the core.

## 7. Who uses what (2025–26)

| Product / system | Model | Notes |
|---|---|---|
| Zep / Graphiti | LPG (Neo4j, FalkorDB, Neptune; Kùzu deprecated) | Facts are `RELATES_TO` edges with `fact`, `valid_at`, `invalid_at`, `expired_at` and `episodes` (a list of UUIDs) |
| Cognee | LPG (Kùzu default, Neo4j, Neptune) + Pydantic DataPoints | Optional OWL ontology resolver (third-party description) |
| Neo4j Agent Memory (Labs, experimental) | LPG | Short-term, long-term (POLE+O) and reasoning memory in one graph |
| Mem0 v3 | No graph in OSS | Graph memory is platform-only (prior research) |
| Microsoft GraphRAG | Relational files (Parquet: entities, relationships, text_units, communities) | Graph semantics over tables |
| Letta | Files + grep | Prior research |
| mycelium | SQLite, claim-centric | Prior research |
| Wikidata | Wikibase statements; RDF export | The reference n-ary statement model |
| Amazon Neptune | Both RDF and LPG; openCypher over RDF in Neptune Analytics | OneGraph research |
| Microsoft Fabric graph | GQL | Closed graph types |
| PostgreSQL 19 / DuckPGQ / Oracle | SQL/PGQ over tables | PG19: no variable-length paths |
| Science (nanopubs) | RDF named graphs | Claim + provenance per unit |

The pattern: agent-memory products are almost all LPG or plain tables, and RDF appears only as optional ontology input. Provenance in those LPG products is weak: id lists on edges, and no spans.

## Leaning (kg-models, not a decision)
- Keep **relational claim tables as the core** (ADR 0009 is already the statement-node pattern). Reasons:
  - It's the only model with FK integrity on supports and claim links at zero extra engine cost.
  - Per-scope recomputation has to be custom in every model anyway.
- Borrow **RDF 1.2 semantics without RDF storage**:
  - claim id = reifier;
  - current = asserted, history = unasserted;
  - make content-hash ids valid `urn:` IRIs;
  - aim the share serializer at TriG / JSON-LD statement nodes (nanopub-like).
- For vocabularies, borrow from **SHACL and SKOS**: report-only cardinality with severities, `skos:definition`/`scopeNote`/`example`. Not OWL.
- Worth a prototype: write ADR 0009's roll-up as a **Datalog program**, for example the Nemo crate, to check whether the rules read better declaratively. Keep `captured_at` as data.
- Avoid edge-as-claim in any LPG design.

## Unverified
- PostgreSQL 19 GA date, and whether SQL/PGQ stayed in the final release (beta 3 on 2026-08-13 had it).
- Whether `n10s` exports relationship properties as RDF 1.2 triple terms or reifiers.
- Oxigraph's current RDF 1.2 / SPARQL 1.2 coverage (`rdf-12` and `sparql-12` cargo features seen in changelog excerpts; 0.5.4 on 2026-01-24 added RDF/XML 1.2).
- JSON-LD WG milestone dates (the draft charters disagree).
- Neo4j's exact list of missing mandatory GQL features; openCypher's standalone status in 2026.
- The PG-Keys venue and year (SIGMOD 2021), taken from the PG-Schema paper's citation; not opened.
- The TypeQL 3 sketch above wasn't run; the `links` syntax is from memory.
- XTDB 2: whether system time can be set on import (matters only if it were used for known time).
- Cognee's OWL resolver details (third-party source).
- Wikidata's current query-service backend.

## Sources (all checked 2026-11-04)
- RDF 1.2 Concepts, CR Snapshot 2026-04-07: https://www.w3.org/TR/rdf12-concepts/
- RDF 1.2 Semantics, CR Draft 2026-09-24: https://www.w3.org/TR/rdf12-semantics/
- RDF 1.2 Turtle, WD 2026-11-01: https://www.w3.org/TR/rdf12-turtle/
- RDF 1.2 Primer, Group Note Draft 2026-09-17: https://www.w3.org/TR/rdf12-primer/
- What's New in RDF 1.2 (suite statuses), 2026-07-14: https://www.w3.org/TR/rdf12-new/
- RDF 1.2 Interoperability, Group Note Draft 2026-07-23: https://www.w3.org/TR/rdf12-interop/
- W3C call for implementations (RDF 1.2): https://www.w3.org/news/2026/w3c-invites-implementations-of-rdf-1-2-concepts-and-abstract-data-model-and-rdf-1-2-semantics
- SPARQL 1.2 Query, WD 2026-11-01: https://www.w3.org/TR/sparql12-query/
- SHACL 1.2 Core, WD 2026-11-02: https://www.w3.org/TR/shacl12-core/ ; SHACL 1.2 Rules, WD 2026-10-31: https://www.w3.org/TR/shacl12-rules/
- SHACL 1.0, Recommendation 2017-07-20: https://www.w3.org/TR/shacl/
- OWL 2 Overview, Recommendation 2012-12-11: https://www.w3.org/TR/owl2-overview/
- PROV-O, Recommendation: https://www.w3.org/TR/prov-o/ ; OWL-Time, CR Draft: https://www.w3.org/TR/owl-time/
- RDF Dataset Canonicalization, Recommendation: https://www.w3.org/TR/rdf-canon/
- R2RML, Recommendation: https://www.w3.org/TR/r2rml/ ; proposed RML WG charter: https://kg-construct.github.io/rml-wg-charter
- JSON-LD 1.1, Recommendation 2020-07-16: https://www.w3.org/TR/json-ld11/ ; JSON-LD WG charter: https://www.w3.org/groups/wg/json-ld/charters/active/
- Tomaszuk & Champin, "Round-Trippable RDF 1.2 Interoperability" (SWJ 2026): https://www.semantic-web-journal.net/content/round-trippable-rdf-12-interoperability
- Nanopublications: https://arxiv.org/pdf/1809.06532 ; multi-source extension (2025): https://link.springer.com/article/10.1007/s00799-025-00431-x
- ISO/IEC 39075 GQL, published 2024-04-12: https://en.wikipedia.org/wiki/Graph_Query_Language (iso.org returned 403)
- Neo4j GQL conformance (updated 2026-04-30): https://neo4j.com/docs/cypher-manual/current/appendix/gql-conformance/
- Microsoft Fabric GQL conformance: https://learn.microsoft.com/fabric/graph/gql-conformance
- PostgreSQL SQL/PGQ commit: https://forger.sitiv.fr/open-dsi/postgres/commit/2f094e7ac69 ; PG19 limits: https://thebuild.com/blog/sqlpgq-in-postgresql-19-graph-queries-without-the-graph-database/ ; Oracle vs PG thread: https://www.postgresql.org/message-id/CAAAe_zBUDtYTKff24aqdviDqJTbch6PSKyuv%2BNNDmEHeBfQDag%40mail.gmail.com
- DuckPGQ: https://duckdb.org/community_extensions/extensions/duckpgq
- PG-Schema (SIGMOD 2023): https://arxiv.org/abs/2211.10962
- Wikibase data model: https://www.mediawiki.org/wiki/Wikibase/DataModel ; RDF statement encoding (KIF paper): https://arxiv.org/pdf/2403.10304
- TypeDB 3.0: https://typedb.com/blog/typedb-3-roadmap ; `@card`: https://typedb.com/docs/typeql-reference/annotations/card/
- Datomic licence: https://blog.datomic.com/2023/04/datomic-is-free.html , https://blog.datomic.com/2023/06/datomic-cloud-is-free.html
- XTDB 2: https://xtdb.com/blog/dev-diary-sep-24
- Nemo: https://github.com/knowsys/nemo , https://arxiv.org/pdf/2308.15897
- OneGraph: https://www.semantic-web-journal.net/node/3273 , https://ceur-ws.org/Vol-3828/paper44.pdf
- neosemantics: https://github.com/neo4j-labs/neosemantics
- Graphiti `edges.py`: https://raw.githubusercontent.com/getzep/graphiti/main/graphiti_core/edges.py
- Cognee graph stores: https://docs.cognee.ai/setup-configuration/graph-stores
- Neo4j Agent Memory: https://neo4j.com/labs/agent-memory , https://pypi.org/project/neo4j-agent-memory/
- GraphRAG outputs: https://microsoft.github.io/graphrag/index/outputs
- Repo: `docs/adr/0009-claims-projection.md:11-25`, `docs/decisions/knowledge.md:11,18,26,28,29,48,49`, `docs/decisions/vocabulary.md:18,36`, `docs/decisions/sharing.md:6-8`
