# 21: Pragmatic architecture (relational claim tables on SQLite)

Agent: `arch-pragmatic`, step 2, 2026-11-05. Stance: claim-as-row in relational tables on SQLite. It is the smallest change from ADR 0009 and the T4/T5 work that still meets every concept requirement. Standards are used only at the edges (export, share). Written without reading `20-arch-semantic.md`. Nothing here is decided; "leaning" marks my preference. Repo paths: `SS` = `crates/rastro-storage-sqlite`, `CORE` = `crates/rastro-core`, `SDK` = `crates/rastro-plugin-sdk/src/lib.rs`, `MIG` = `SS/migrations`; `T4:`/`T5:` prefix = the worktree `../rastro-ai.worktrees/claims-t4|claims-t5`.

Working names that are **not glossary terms** (implementation only; none is meant for the glossary): *lens* (one registered scope set and its materialized state), *slot* (one support stream: source, scope, location, field, rule, extractor version), *field group* (every slot observing one subject, predicate, location and field), *anchor* (the type claim a candidate's other claims depend on), *cluster* (13-identity.md's term).

**Correction to `00-current-state.md` §3:** T5's `code` vocabulary *does* declare `cardinality = "one"` (`T5:crates/rastro-extractor-github/vocabulary.toml:69,83,97`). T5's SDK `Predicate` has no such field (`T5:SDK:404-416`), and nothing denies unknown fields, so serde drops it without a word. After a T4 merge it parses as `One`, so the feared "code:state never supersedes" doesn't happen. The real lesson is that vocabulary parsing must reject unknown fields.

---

## 1. Design summary
1. **Store:** SQLite stays the only engine (ADR 0002). No graph or RDF engine runs inside. RDF 1.2 / JSON-LD appear only in export and share serializers.
2. **Claim = row**, with an id that is a content hash over (subject, predicate, object) and is **never re-keyed** (ADR 0009, D5). Literals are canonicalised per datatype before hashing.
3. **`claim_support` becomes `justification`**, of kind provenance, inference or confirmation. AND-premises sit in `justification_premise`; several justifications on one claim are OR (JTMS-style).
4. **Time sits at two levels.** The base layer keeps scope-independent, slot-local support intervals. Each lens keeps **claim intervals as occurrence records** carrying `opened_by` and `closed_by` (evidence, intent, claim). A→B→A gives three occurrences, `superseded_by` stays acyclic, and history needs no cycle of links.
5. **Precious inputs grow by two tables:** `intent` (the owner's confirm, reject, answer and undo) and `extraction_artifact` (recorded output of non-deterministic extraction). Both are insert-only, outside every generation, and exported. The artifact table needs an amendment to knowledge §12.
6. **Rebuild ≠ re-extract.** Rebuild replays evidence (deterministic extractors re-run), artifacts and intents with no model call. Re-extraction is an explicit command that writes new artifacts.
7. **Scopes are handled by *lenses*.** Every scope set a writing open launches with (`*`, `personal`, `work`…) gets its derived state materialized: status, revision, intervals, links, adjacency, types, aliases, clusters and degrees. Scope semantics are non-monotone (a hidden newer observation must not hide a visible one), so this is the cheapest way to keep D18/S28 exact with O(limit) reads.
8. **Identity uses 13-identity.md's option C plus a Person hub (P2).** Core meta-predicates: `rastro:type`, `rastro:alias`, `rastro:held_by` (account → Person), `rastro:same_as` (Person ↔ Person and same-kind duplicates) and `rastro:distinct_from` (cannot-link). Clusters are computed per lens. Undo is a reject intent that survives rebuild.
9. **Retrieval** uses fixed operations: `about`, `why` and `search`, plus additive, bounded `expand` options. One-hop reads are an index range scan; a two-hop walk is a budgeted BFS in Rust with per-lens hub cut-offs. No global statistic feeds a scoped answer.
10. **T4/T5 are kept in substance, not merged as they are.** One new migration replaces both projection schemas. T4's full-replay `refresh` becomes the test oracle, and an incremental per-lens recompute takes its place. T5's in-place `entity.type` and `alias` mutations become lens-derived state.

### Where this stance hurts (stated up front)
| Weakness | Concretely | Mitigation, and what it doesn't fix |
|---|---|---|
| Interop | No native RDF; ids, export and share need a hand-written serializer and its tests | `urn:rastro:` IRIs from content ids; a JSON-LD 1.1 statement-node shape works today (10-models.md §6). Each format we support is code we own |
| Schema validation | Domain, range, subtyping, cardinality and alias kinds are Rust code, with no SHACL engine; vocabulary semantics can drift from the code | A fixed, small set of vocabulary features plus conformance fixtures; SHACL/SKOS go to *export* only. Anything outside the fixed set needs core code |
| Ad-hoc graph queries | No Cypher or SPARQL for agents; paths deeper than 2 hops are application code | An `edge` SQL view for the owner, `rastro export --rdf` into any SPARQL tool, and the fixed MCP operations. Agents can't ask arbitrary graph questions |
| Hand-rolled temporal logic | The interval sweep, outranking and status precedence are about 500 lines of our own Rust; interval arithmetic is bug-prone | T4's naive full replay kept as the oracle, a property test of incremental ≡ rebuild, and the scope-isolation oracle (§6). Bugs are found by tests, not prevented by an engine |
| Lens multiplication | Derived state is stored once per registered scope set: about (1 + distinct scopes per claim) × claims | Lenses are registered only by launches; unused ones are garbage-collected. Many distinct scope sets would multiply storage |
| Rebuild publication | T4's copy-in-one-transaction is O(projection) and doubles the WAL at publication | Fine at about 10⁵ claims; §6 gives the trigger for moving to generation-keyed tables |

---

## 2. Per area

### A1 Data model
| | |
|---|---|
| **Proposal** | Relational statement-node (n-ary) tables, as ADR 0009 already has them (10-models.md "Bottom line"). `claim(id, subject, predicate, object_entity \| object_value+datatype, scalar)`: binary S-P-O, with no qualifiers on the claim row. Everything n-ary hangs off the claim id: justifications, premises, intervals, links, state. Valid time, once known, is a justification attribute (`valid_from/to`) rolled up per lens. A contextual qualifier ("Ana's role *at Acme*") is modelled as an entity (a Role or Membership) rather than a qualifier column. |
| **Rejected** | **RDF store:** standards still at WD/CR (10-models.md "Bottom line"), cross-process readers undefined in Oxigraph (12-engines.md transactions table), and the per-scope recomputation is custom anyway. **LPG engine:** claim-as-node anyway (10-models.md §2), no FK integrity, engine churn (Kùzu archived; 12-engines.md "Constraints"). **Edge-as-claim:** breaks A2 (10-models.md §2). **Hypergraph/TypeDB:** a separate server, and `@card` rejects contradictions (10-models.md §4). **Qualifier columns on claim:** they would change the claim id rule or collide (11-time-provenance.md "Identity issue"). |
| **Cost** | Every graph feature is hand-built. One logical hop is two joins (the adjacency table removes this on reads). |
| **Keeps / amends / contradicts** | Keeps knowledge §1, §5; ADR 0002; ADR 0009 ids. **Amends ADR 0009:** `entity.type` moves into `rastro:type` claims; the "tables" list changes (§3). |
| **Owner question** | Are entity-ised qualifiers (Role, Membership) acceptable, instead of qualifiers on claims? (Q11) |

### A2 Reification and provenance
| | |
|---|---|
| **Proposal** | `justification(kind ∈ provenance\|inference\|confirmation)`, one row per reason, with `justification_premise` for AND-premises. Provenance carries the evidence, span, rule, extractor version and (for non-deterministic rules) the artifact. Inference carries a core rule and its premises. Confirmation carries the intent. **A provenance justification may also have premises**: the claims of one extraction candidate depend on its anchor (§4c), so rejecting a commitment carries through to its slots. Status is OR over live justifications; a justification is live only if all its premises are live (11-time-provenance.md "Justifications and retraction"). Inference rules are **core-only, stratified and acyclic** for now: a fixed registry in Rust, each rule with a version. |
| **Rejected** | **Supports only, with confirmations as a status column:** that loses "why confirmed" and the carry-through. **Separate tables per kind:** three copies of liveness and visibility code. **Plugin-defined inference ("deriver" plugins):** that needs a claims-in protocol now, which is premature. **Cyclic inference with fixpoint maintenance:** a self-supporting cycle (11-time-provenance.md, last paragraph of that section). |
| **Cost** | One wide table with CHECKs per kind; carry-through walks `premise_by_claim`. |
| **Keeps / amends / contradicts** | Keeps the glossary (Justification), knowledge §5, §6, §10; renames ADR 0009's "claim support" (amendment). |
| **Owner question** | Does an inferred claim over observed or confirmed premises with a deterministic rule read `observed`, or something else? (Q8) |

### A3 Temporal model
| | |
|---|---|
| **Proposal** | **Base layer (scope-independent):** each provenance justification is one *occurrence of support in a slot*: `start_at` = `captured_at` of the establishing evidence, `end_at` = `captured_at` of the next observation in the same slot that yields a different claim, plus `ended_by_evidence`. A slot is local to one (source, scope) pair, so it only ever appends in replay order (`SS/src/lib.rs:303-312`). **Lens layer:** a sweep over visible justifications in a field group gives `claim_interval(start, end, opened_by_*, closed_by_*, closed_by_claim)`. **`superseded_by(Y) = closed_by_claim` of Y's latest interval**: a function, acyclic because it always points to a claim whose interval started later (proof sketch in §3). `conflicts_with` holds between live claims of a scalar field group whose `source_updated_at` are tied or missing. Valid time: `valid_basis = 'unknown'` until evidence carries dates (slice 5); never derived from an item's update time. **"As of t" (slice 7)** = `claim_interval` covering t in the lens: "what today's rules derive from evidence captured by t". "What did Rastro *serve* on Tuesday" is **not** supported; only a cheap `projection_publication` log of extractor and vocabulary versions per generation is kept. |
| **Rejected** | **Claim-level `known_interval` keyed (claim, evidence) (T4):** it can't be scoped and has no transition. **Accumulated historical `supersedes` links:** they form an O↔C cycle (11-time-provenance.md, concrete example). **Contextual claim ids (time inside the id):** that breaks D5 and renderer targets. **SQL:2011 or XTDB system time:** the engine's clock is not `captured_at` (10-models.md Datalog/EAV entry; 11-time-provenance.md time models). **Full answer receipts:** storage plus versioning cost for an audit nobody asked for yet. |
| **Cost** | A hand-written sweep (about 150 lines). Intervals are stored once per lens. Delayed extraction means an interval can start before a reader could have seen it (accepted; 11-time-provenance.md "Two different 'as of' questions"). |
| **Keeps / amends / contradicts** | Keeps knowledge §7, §25, §26; D6; S9, S12. **Amends ADR 0009** "known interval: claim, start, end" → per-lens occurrences with transitions. |
| **Owner question** | Is "as of = re-derived under today's rules" enough, or must served answers be reproducible? (Q9) |

### A4 Identity and merges
| | |
|---|---|
| **Proposal** | **Overlay, never re-key** (13-identity.md §3 option C). An entity id is the hash of its canonical identifier, as on main. **Person hub (13-identity.md §6, P2):** a Person is minted from an exact identifier (`person:email/<normalised, provider-versioned>`), from an owner intent (`person:intent/<intent id>`), or from an address-book record. Accounts link to it through `rastro:held_by`, whose justification is provenance (a source-vouched identifier, deterministic rule) or confirmation (an Ask answer). Two accounts sharing a vouched email reach the *same* Person id with no merge at all (knowledge §24 extended). **`rastro:same_as`** (symmetric id: min, max) joins two Persons, or two same-kind entities (a transferred issue), from an exact rule or a confirmation. **Clusters** = connected components of live `same_as`, computed per lens; representative = min(member id) among *visible* members. **Undo** = a `reject` intent on the statement (the cannot-link), so rebuild never re-derives it and the Ask is never repeated (levels §1). A `distinct_from` inside a cluster is a conflict and becomes an Ask. Auto-merge happens only on vouched identifiers, with a blocklist (`noreply@`, role mailboxes) and a per-type limit (13-identity.md §5). **"One claim, two supports" across members** is grouped at read time by (predicate, cluster(subject), cluster(object) or literal). |
| **Rejected** | **Re-keying (option D):** claim ids churn on merge and on unmerge, and renderer targets break or need redirects (13-identity.md §3, §9). **Destructive merge or union-find:** no undo, order-dependent. **Merging accounts into each other (P1):** an account is not a human (vocabulary §12). **Person = cluster (P3):** a shared account glues two humans together. |
| **Cost** | Read-time grouping for `expand: identity`. Cluster recomputation per lens is cheap (components are small). A Person minted from a vouched identifier can change id when the email changes (that is just two Persons and a later `same_as`). |
| **Keeps / amends / contradicts** | Keeps knowledge §4, §24, §30; renderers §5 (ids stable). **Glossary tension, explicit:** "Merge: a claim that two entities are one." Under P2, "these two accounts are one human" is two `held_by` claims to one Person, not a claim that the accounts are one entity. Either the glossary entry broadens ("…are one, or belong to one Person") or Merge stays `same_as`-only and the owner-facing word for the account case is "link". **Amends vocabulary §1:** adds core meta-model predicates in a `rastro:` namespace. |
| **Owner question** | Q3 (P2 and the Merge wording), Q5 (which scope an identity answer carries), Q12 (is undo always a cannot-link?). |

### A5 Vocabulary and schema
| | |
|---|---|
| **Proposal** | Keep T5's TOML format with its required definition fields (`T5:SDK:373-440`) and add: `cardinality` **required** (no T4 default of `many`), an optional `supertype`, `#[serde(deny_unknown_fields)]`, and a fixed datatype registry (`string`, `integer`, `decimal`, `boolean`, `date`, `datetime`, `duration`, `iri`, vocabulary enums) with **canonical lexical forms before hashing**, so `2026-11-06` and `2026-11-06T00:00Z` don't become two claims. Installed vocabularies are loaded into `vocabulary_term` and `vocabulary_endpoint` per generation, with the file hash in `projection_publication`. Core meta-model in the `rastro:` namespace (type, alias, held_by, same_as, distinct_from), defined by the core with the same definition fields. Validation in Rust: domain and range with supertype closure; **type grounding** (a submitted endpoint type must be backed by a live `rastro:type` claim in this run or the base, which is why T5's type-first ordering stays); cardinality is **report-only**, so a scalar field group with two live values is a conflict, never a rejection (vocabulary §13). Cross-namespace use: a manifest `dependencies` entry may name an installed vocabulary plugin (`std`), loaded read-only. Overlap mappings (vocabulary §7) are claims between term entities (`rastro:term/<term>`), justified only by confirmation. A vocabulary version change means replay: deterministic extractors re-run; artifacts are *re-validated* against the new version; artifacts produced under a changed *definition* are reported `behind` until the owner re-extracts. |
| **Rejected** | **SHACL engine or RDFS inside:** an RDF stack for a dozen checks, and RDFS domain and range *infer* rather than reject (10-models.md §1). **TypeDB-style enforcing cardinality:** contradicts vocabulary §13. **Automatic vocabulary growth from LLM output:** governance breach (15-llm-ontology.md §5). |
| **Cost** | Each new vocabulary feature is core code plus fixtures. Export to SKOS/SHACL is an extra serializer. |
| **Keeps / amends / contradicts** | Keeps vocabulary §3–§7, §13. **Amends ADR 0008** (vocabulary file format, meta-predicates, type grounding). T5's un-namespaced `type` and `alias` (`T5:CORE/src/claims.rs:16-21`) are renamed: their claim ids change, which is free now because no stored data exists (`00-current-state.md` §4). |
| **Owner question** | Re-validate or re-extract when a vocabulary's definition changes? (Q2b) |

### A6 Traversal and indexes
| | |
|---|---|
| **Proposal** | A per-lens **`adjacency`** table: one row per (entity, claim, direction), keyed `(lens, entity, known_since DESC, claim, direction)`, so `about` is one ordered index range scan and stops at `limit + 1`, whatever the degree (checked: a 20k-row hub returns 21 rows from the primary key, no sort; §4d). `adjacency_by_predicate` serves per-predicate caps for expansion. **Two hops:** a BFS in Rust over the adjacency index with **per-(entity, predicate, direction) cap k**, a **hub cut-off on `entity_state.degree_*` of the same lens** (never a global degree), a global examined-rows budget, and `truncated_reasons` in the result (14-retrieval.md "Traversal and ranking experiments", items 3–4). The seed entity is never cut off, so "the bot is the answer" still works (14-retrieval.md, 10k-claim bot example). Recursive CTEs only for the bounded `why --premises` walk (depth ≤ number of strata). No closure table. |
| **Rejected** | **Recursive CTE as the main traversal:** unbounded work before `LIMIT` (14-retrieval.md "Index design", two-hop row). **Closure or reachability table:** O(V²), per lens, weak for 1–2 hops. **PPR or community summaries:** no measured benefit for this owner yet, and a global computation leaks across scopes. |
| **Cost** | `adjacency` rows ≈ 2 × entity-valued claims + 1 × literal claims, per lens. |
| **Keeps / amends / contradicts** | Keeps knowledge §15; ADR 0002 ("1–2 hop walks"); D15 order. |
| **Owner question** | The hub threshold and per-predicate caps: fixed defaults, or set per vocabulary? (Q10) |

### A7 Store and engine
| | |
|---|---|
| **Proposal** | **SQLite only**, behind the existing `Storage`/`ReadStorage` traits. Generation handling: T4's TEMP shadow tables, swapped in by DELETE + INSERT inside one transaction (`T4:SS/src/lifecycle.rs:233-284`). Under WAL a reader sees the old or the new generation, never a mix (ADR 0009). An append-only `projection_publication` log sits outside the generation. Optional, disposable sidecars only if a benchmark fails (12-engines.md "Hybrid and rebuild patterns", option E): DuckDB + DuckPGQ attached read-only to a published snapshot for analytics, or Oxigraph loaded from the RDF export for SPARQL. Neither is ever authoritative. |
| **Rejected** | **An embedded graph engine now** (Ladybug, Grafeo, Helix): single-writer and process constraints break the reader beside the writer (12-engines.md transactions table), plus maintenance risk. **Generation-keyed tables (a `generation` column on every row):** every index gets a prefix and reads gain a predicate. Kept as the fallback if publication cost fails its test (§6, R10). **A separate database file per generation:** WAL transactions aren't atomic across attached files. |
| **Cost** | Publication is O(projection) (copy). The WAL peaks at about 2× projection during publication. |
| **Keeps / amends / contradicts** | Keeps ADR 0002 (re-affirms "graph DB rejected"), ADR 0009 atomicity. |
| **Owner question** | Which latency and size numbers would justify a sidecar? (§6 gives defaults) |

### A8 Several extractors on one entity
| | |
|---|---|
| **Proposal** | `source_extractor(source, extractor)` replaces the single `sources.extractor` column (config projection; ADR 0008 binding unchanged). The claim id excludes the extractor, so equal statements from two extractors are **one claim with two justifications** (knowledge §5). The **field group excludes the rule**, so two extractors can't hide a disagreement behind different rule names (`T4:SS/src/lifecycle.rs:22-31` already does this). Authority = the predicate's vocabulary default (`newer_comparable_observation` today); an owner override is a *rule* (glossary: Rule), a precious input applied in the lens sweep. **Version retirement:** once a new extractor version's checkpoint catches up on a pair, the old version's justifications for that pair are *deleted*, not marked. Incremental then equals rebuild, which only knows bound versions. A claim that only the old version supported disappears unless an intent sustains it. Revision keys exclude `extractor_version`, so an upgrade that reproduces identical supports leaves revisions unchanged. |
| **Rejected** | **Extractor inside the claim id:** duplicates (knowledge §5). **Keeping old-version supports forever (main, T4):** rebuild and incremental diverge. **Authority by plugin rank:** knowledge §9 says per kind of claim. |
| **Cost** | During a transition both versions' supports count, so status is their union; documented. |
| **Keeps / amends / contradicts** | Keeps knowledge §5, §8, §9, §26; plugins §15. Amends main's one-extractor-per-source (`CORE/src/runs/mod.rs:107-118`, de facto only). |
| **Owner question** | Confirm "delete on catch-up" for retired versions (Q13). |

### A9 Hypothesis and confirmed claims
| | |
|---|---|
| **Proposal** | **Hypothesis path (slice 5):** a rule declared `deterministic = false` may cite `unstructured` evidence, and `untrusted content` too if the owner allows it (Q7). It goes through the same core validation plus an **exact UTF-8 byte quote check** (15-llm-ontology.md §3). Its claims are `hypothesis`, never `observed`; the submission can't choose its status. **Intents are precious:** `intent` + `intent_target` hold the full statement, not just the claim id, so a confirmation can recreate its claim with no extractor ("knowledge = f(evidence, confirmations, rules)"). An intent is checked against the revision the owner saw **once, when written** (renderers §5); it is not re-checked at rebuild. **Status precedence per lens:** see §3. **Carry-through:** a retracted claim makes every justification with it as premise not live; the claim keeps any other live justification (OR); events list the changed claims; the execution graph flags Action runs and proposals whose basis (claim id, revision) changed (knowledge §10). **A confirmation sent as one batch over a candidate is conditioned on the anchor**, so rejecting a commitment retracts its confirmed slots too (§4c). |
| **Rejected** | **Confirmations as evidence rows from an "owner" source:** that reuses insert-only storage, but blurs provenance with confirmation and lets a deterministic rule turn owner text into `observed`. **Blanket descendant retraction:** throws away valid alternative support (11-time-provenance.md). **Status column set by intents in place:** not rebuildable. |
| **Cost** | Two precious tables (export, forgetting, scope). The precedence rules are code. |
| **Keeps / amends / contradicts** | Keeps knowledge §5, §10, §12, §23 (the observed path stays strict); levels §1, §2, §9. Needs the **new hypothesis acceptance rule** that ADR 0008 anticipates ("Consequences"). |
| **Owner question** | Q4 (does a confirmation sustain a claim alone, does a newer observation supersede a confirmed scalar, does a reject block later evidence), Q6 (anchor conditioning), Q7. |

### A10 Retrieval for agents
| | |
|---|---|
| **Proposal** | Fixed operations only, all read-only and bounded, all reading the caller's lens: `about(entity, since?, limit?, history?, expand?)`, where `expand ∈ none (default; D15 unchanged) \| identity` (the cluster plus accounts `held_by` the same Person, then a k-way merge of per-member top-k pages); `why(claim, limit?, full?, premises?)`; and `search(text, …, claims?: bool)`, which attaches claims *supported by* the hit evidence (a citation lookup, not a hop) with a separate budget. Ranking signals come only from lens state: visible support count (`live_count`), lens degree and known time. FTS rank leakage stays the accepted limit (surfaces §21); the graph adds **no new global statistic**. Agents get no SQL, Cypher or SPARQL; the owner gets the `edge` view in the CLI and the RDF export. |
| **Rejected** | **A query language on MCP:** no row-level security, unbounded work (14-retrieval.md "Agent query surface"). **PPR or summary routing:** unmeasured, and a global computation leaks across scopes. |
| **Cost** | Several round trips for deep questions; an agent can't express "count all X". |
| **Keeps / amends / contradicts** | Keeps surfaces §22–§26; D15–D18. `expand` and `claims?` are additive parameters, a surfaces amendment. |
| **Owner question** | Q10 (expansion surface), Q14 (any owner-facing ad-hoc query surface beyond export). |

### A11 Fate of T4/T5
| | |
|---|---|
| **Proposal** | **Rework both onto one new migration; merge neither as it is** (details in §5). Keep: T4's lifecycle semantics, rebuild mechanism and `ClaimView` fields; T5's extractor crate, URL rules, vocabulary format, meta-claims and alias-quote check. Rework: T4's O(N)-per-batch `refresh`, hard-coded `Source::github`, revision encoding under the v1 tag and `valid_time` CHECK; T5's in-place type and alias mutation, un-namespaced meta-predicates and undocumented location context. |
| **Rejected** | **Merge as they are, then refactor:** the merge doesn't compile (`00-current-state.md` §3), and T4's migration would become an applied file we can never edit (ADR 0002). **Drop and restart:** both hold correct, tested semantics worth about 2.5k lines. |
| **Cost** | About two reworked tickets (T4′, T5′) before T6–T8 (estimate). |
| **Keeps / amends / contradicts** | Keeps the plan's D1–D24, except D5's revision encoding (new tag) and D13's mechanism. |
| **Owner question** | Q1 (lens) gates T4′; Q3 gates the meta-predicate names in T5′. |

---

## 3. Concrete schema

Checked: the DDL below parses in `sqlite3` (3.4x, `:memory:`), and the hot-path plans in §4d were taken from it. One new migration (e.g. `MIG/2026110xxxxx_knowledge_v2.sql`) **drops** main's T3 projection tables (`entity`, `alias`, `claim`, `claim_support`, `extraction_checkpoint`), which are disposable, and creates these. `sources.extractor` stays as a deprecated column, since migrations are never edited. T4's unmerged `202611061100_claim_lifecycle.sql` is never applied.

```sql
-- ===== A. Precious inputs: outside every generation, insert-only, exported (sharing §1) =====
CREATE TABLE intent (                      -- glossary: Intent; a 'confirm' is a Confirmation
  id          TEXT PRIMARY KEY,            -- i_ + H("rastro/intent/v1", kind, created_at, nonce): stable across export
  kind        TEXT NOT NULL CHECK (kind IN ('confirm','reject','answer','undo')),
  created_at  TEXT NOT NULL,               -- its known time
  scope       TEXT NOT NULL,               -- the scope it carries (Q5)
  surface     TEXT NOT NULL,               -- cli | mcp | vault | herdr …
  undoes      TEXT REFERENCES intent(id),
  answer      TEXT,                        -- for 'answer': the Ask's chosen option, mapped to targets below
  CHECK ((kind = 'undo') = (undoes IS NOT NULL))
);
CREATE TABLE intent_target (               -- the full statement, so the claim is rebuildable from the intent alone
  intent_id  TEXT NOT NULL REFERENCES intent(id),
  ordinal    INTEGER NOT NULL,
  claim_id   TEXT NOT NULL,                -- content id; may be absent from the current generation
  revision   TEXT,                         -- what the owner saw (renderers §5); NULL for a new statement
  subject_identifier TEXT NOT NULL, subject_type TEXT NOT NULL, predicate TEXT NOT NULL,
  object_identifier TEXT, object_type TEXT, object_value TEXT, object_datatype TEXT,
  PRIMARY KEY (intent_id, ordinal),
  CHECK ((object_identifier IS NULL) = (object_value IS NOT NULL AND object_datatype IS NOT NULL))
) WITHOUT ROWID;
CREATE INDEX intent_target_by_claim ON intent_target (claim_id);
CREATE INDEX intent_by_undoes ON intent (undoes) WHERE undoes IS NOT NULL;
CREATE TRIGGER intent_no_update BEFORE UPDATE ON intent BEGIN SELECT RAISE(ABORT,'intent is insert-only'); END;
CREATE TRIGGER intent_no_delete BEFORE DELETE ON intent BEGIN SELECT RAISE(ABORT,'intent is insert-only'); END;
-- (same pair of triggers on intent_target; forgetting, slice 14, gets a marker path like evidence)

CREATE TABLE extraction_artifact (         -- needs knowledge §12 amendment (Q2)
  id TEXT PRIMARY KEY,                     -- a_ + H(request identity)
  extractor TEXT NOT NULL, extractor_version TEXT NOT NULL,
  source TEXT NOT NULL, scope TEXT NOT NULL,             -- one pair per run (ADR 0008)
  vocabulary_hash TEXT NOT NULL, prompt_hash TEXT NOT NULL,
  model TEXT NOT NULL, settings TEXT NOT NULL,           -- JSON: model revision, decoder, temperature, seed
  request_hash TEXT NOT NULL,
  response BLOB NOT NULL,                  -- raw bytes as received
  submissions TEXT NOT NULL,               -- JSON claim.submit list as parsed: the replay input
  created_at TEXT NOT NULL,
  run_id INTEGER NOT NULL REFERENCES runs(id)
);
CREATE UNIQUE INDEX artifact_by_request ON extraction_artifact (extractor, extractor_version, request_hash);
CREATE TABLE artifact_input (
  artifact_id TEXT NOT NULL REFERENCES extraction_artifact(id),
  ordinal INTEGER NOT NULL, evidence_id INTEGER NOT NULL REFERENCES evidence(id),
  PRIMARY KEY (artifact_id, ordinal)
) WITHOUT ROWID;
CREATE INDEX artifact_input_by_evidence ON artifact_input (evidence_id);   -- forgetting carries through

-- ===== Config projection and logs (outside the generation) =====
CREATE TABLE source_extractor (source TEXT NOT NULL REFERENCES sources(name), extractor TEXT NOT NULL,
  PRIMARY KEY (source, extractor)) WITHOUT ROWID;
CREATE TABLE lens (                        -- registered by a writing open with that launch scope set
  id INTEGER PRIMARY KEY,
  scopes TEXT NOT NULL UNIQUE,             -- canonical sorted JSON array; '["*"]' = every scope
  registered_at TEXT NOT NULL, last_used_at TEXT NOT NULL
);
CREATE TABLE projection_publication (      -- what rules a generation was built with (cheap audit, A3)
  generation INTEGER PRIMARY KEY, published_at TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('rebuild','incremental')),
  evidence_max_id INTEGER NOT NULL, intent_count INTEGER NOT NULL,
  extractors TEXT NOT NULL, vocabularies TEXT NOT NULL      -- JSON {id: version}, {namespace: file hash}
);

-- ===== B. Projection, base layer: scope-independent; in the generation =====
CREATE TABLE vocabulary_term (
  term TEXT PRIMARY KEY,                   -- 'code:state', 'rastro:held_by', 'code:PullRequest', 'date'
  kind TEXT NOT NULL CHECK (kind IN ('type','predicate','alias_kind','datatype')),
  namespace TEXT NOT NULL, version TEXT NOT NULL, file_hash TEXT NOT NULL,
  supertype TEXT,
  cardinality TEXT CHECK (cardinality IN ('one','many')),
  authority TEXT, freshness TEXT, sensitivity TEXT,
  definition TEXT NOT NULL                 -- JSON: definition, not_to_confuse_with, examples, counter_examples, evidence_requirement
) WITHOUT ROWID;
CREATE TABLE vocabulary_endpoint (
  predicate TEXT NOT NULL REFERENCES vocabulary_term(term),
  role TEXT NOT NULL CHECK (role IN ('domain','range')), term TEXT NOT NULL,
  PRIMARY KEY (predicate, role, term)
) WITHOUT ROWID;

CREATE TABLE entity (                      -- no type column: types are rastro:type claims (A1, A5)
  id TEXT PRIMARY KEY,                     -- e_ + H("rastro/entity/v1", identifier): unchanged
  identifier TEXT NOT NULL UNIQUE
);
CREATE TABLE claim (
  id TEXT PRIMARY KEY,                     -- c_ + H("rastro/claim/v1", …): unchanged; same_as uses (min, max)
  subject TEXT NOT NULL REFERENCES entity(id),
  predicate TEXT NOT NULL,
  object_entity TEXT REFERENCES entity(id),
  object_value TEXT, object_datatype TEXT, -- value in the datatype's canonical lexical form
  scalar INTEGER NOT NULL CHECK (scalar IN (0,1)),
  CHECK ((object_entity IS NULL) = (object_value IS NOT NULL AND object_datatype IS NOT NULL))
);
CREATE INDEX claim_by_subject ON claim (subject, predicate);
CREATE INDEX claim_by_object  ON claim (object_entity, predicate) WHERE object_entity IS NOT NULL;
CREATE INDEX claim_by_value   ON claim (predicate, object_value)  WHERE object_value IS NOT NULL;

CREATE TABLE justification (
  id INTEGER PRIMARY KEY,                  -- row id: never exported, never shown as an id
  jkey TEXT NOT NULL UNIQUE,               -- H(kind, evidence, span, rule, extractor_version | intent | rule+premises)
  claim_id TEXT NOT NULL REFERENCES claim(id),
  kind TEXT NOT NULL CHECK (kind IN ('provenance','inference','confirmation')),
  scope TEXT,                              -- provenance: evidence.scope; confirmation: intent.scope; inference: NULL (from premises)
  evidence_id INTEGER REFERENCES evidence(id), field TEXT, start INTEGER, "end" INTEGER,
  rule TEXT, deterministic INTEGER CHECK (deterministic IN (0,1)),
  extractor TEXT, extractor_version TEXT, run_id INTEGER REFERENCES runs(id),
  artifact_id TEXT REFERENCES extraction_artifact(id),
  intent_id TEXT REFERENCES intent(id),
  slot TEXT,                               -- H(source, scope, location, field, rule, extractor_version [, claim if not scalar])
  field_key TEXT,                          -- H(subject, predicate, location, field): the field group, scalar only
  source_updated_at TEXT,                  -- comparability clock (D13)
  valid_from TEXT, valid_to TEXT,          -- only when evidence states them (slice 5); never from item update time
  start_at TEXT NOT NULL,                  -- slot-local: captured_at of the establishing evidence, or intent time
  end_at TEXT,                             -- slot-local: captured_at of the replacing observation in this slot
  ended_by_evidence INTEGER REFERENCES evidence(id),
  end_reason TEXT CHECK (end_reason IN ('replaced','undone')),
  CHECK (kind <> 'provenance'   OR (evidence_id IS NOT NULL AND scope IS NOT NULL AND slot IS NOT NULL
                                    AND rule IS NOT NULL AND "end" > start)),
  CHECK (kind <> 'confirmation' OR (intent_id IS NOT NULL AND scope IS NOT NULL)),
  CHECK (kind <> 'inference'    OR (rule IS NOT NULL AND scope IS NULL)),
  CHECK ((end_at IS NULL) = (end_reason IS NULL))
);
CREATE INDEX justification_by_claim    ON justification (claim_id, scope, end_at);
CREATE INDEX justification_by_evidence ON justification (evidence_id) WHERE evidence_id IS NOT NULL;
CREATE INDEX justification_by_slot     ON justification (slot, start_at) WHERE slot IS NOT NULL;
CREATE INDEX justification_by_field    ON justification (field_key)     WHERE field_key IS NOT NULL;
CREATE INDEX justification_by_intent   ON justification (intent_id)     WHERE intent_id IS NOT NULL;
CREATE INDEX justification_by_artifact ON justification (artifact_id)   WHERE artifact_id IS NOT NULL;
CREATE TABLE justification_premise (     -- AND: live only while every premise is live in the lens
  justification_id INTEGER NOT NULL REFERENCES justification(id),
  ordinal INTEGER NOT NULL,
  premise_claim_id TEXT NOT NULL REFERENCES claim(id),
  PRIMARY KEY (justification_id, ordinal)
) WITHOUT ROWID;
CREATE INDEX premise_by_claim ON justification_premise (premise_claim_id);   -- carry-through walk

CREATE TABLE extraction_checkpoint (     -- unchanged from main
  extractor TEXT NOT NULL, extractor_version TEXT NOT NULL, source TEXT NOT NULL, scope TEXT NOT NULL,
  last_captured_at TEXT NOT NULL, last_evidence_id INTEGER NOT NULL,
  PRIMARY KEY (extractor, extractor_version, source, scope)
) WITHOUT ROWID;

-- ===== C. Projection, lens layer: one copy per registered scope set; in the generation =====
CREATE TABLE justification_current (lens INTEGER NOT NULL REFERENCES lens(id),
  justification_id INTEGER NOT NULL REFERENCES justification(id),
  PRIMARY KEY (lens, justification_id)) WITHOUT ROWID;
CREATE TABLE claim_state (               -- exists ⇔ the claim has ≥1 justification visible in the lens
  lens INTEGER NOT NULL REFERENCES lens(id), claim_id TEXT NOT NULL REFERENCES claim(id),
  status TEXT NOT NULL CHECK (status IN ('observed','hypothesis','confirmed','retracted','superseded')),
  revision TEXT NOT NULL,                -- r_ + H("rastro/revision/v2", …), see below
  known_since TEXT NOT NULL,             -- start of the latest interval (D15 sort key)
  live_count INTEGER NOT NULL,           -- visible current justifications
  valid_from TEXT, valid_to TEXT,
  valid_basis TEXT NOT NULL DEFAULT 'unknown' CHECK (valid_basis IN ('unknown','evidence','inferred')),
  PRIMARY KEY (lens, claim_id)
) WITHOUT ROWID;
CREATE TABLE claim_interval (            -- occurrences, with the transition that opened and closed each
  lens INTEGER NOT NULL REFERENCES lens(id), claim_id TEXT NOT NULL REFERENCES claim(id),
  start TEXT NOT NULL, "end" TEXT,
  opened_by_evidence INTEGER REFERENCES evidence(id), opened_by_intent TEXT REFERENCES intent(id),
  closed_by_evidence INTEGER REFERENCES evidence(id), closed_by_intent TEXT REFERENCES intent(id),
  closed_by_claim TEXT REFERENCES claim(id),
  PRIMARY KEY (lens, claim_id, start)
) WITHOUT ROWID;
CREATE TABLE claim_link (                -- current links only; history lives in claim_interval
  lens INTEGER NOT NULL REFERENCES lens(id),
  from_claim TEXT NOT NULL REFERENCES claim(id),
  kind TEXT NOT NULL CHECK (kind IN ('conflicts_with','supersedes')),
  to_claim TEXT NOT NULL REFERENCES claim(id),
  PRIMARY KEY (lens, from_claim, kind, to_claim)
) WITHOUT ROWID;
CREATE INDEX claim_link_inverse ON claim_link (lens, to_claim, kind);
CREATE TABLE adjacency (                 -- the one-hop index; O(limit) about() on any hub
  lens INTEGER NOT NULL REFERENCES lens(id), entity_id TEXT NOT NULL REFERENCES entity(id),
  known_since TEXT NOT NULL, claim_id TEXT NOT NULL REFERENCES claim(id),
  direction TEXT NOT NULL CHECK (direction IN ('out','in')),
  predicate TEXT NOT NULL,
  live INTEGER NOT NULL CHECK (live IN (0,1)),    -- 1: observed | hypothesis | confirmed
  PRIMARY KEY (lens, entity_id, known_since DESC, claim_id, direction)
) WITHOUT ROWID;
CREATE INDEX adjacency_by_predicate ON adjacency (lens, entity_id, direction, predicate, live, known_since DESC);
CREATE TABLE entity_state (              -- exists ⇔ the entity is visible in the lens (S22)
  lens INTEGER NOT NULL REFERENCES lens(id), entity_id TEXT NOT NULL REFERENCES entity(id),
  types TEXT NOT NULL,                   -- JSON of live rastro:type values (two = a visible conflict)
  display_alias TEXT,
  degree_in INTEGER NOT NULL, degree_out INTEGER NOT NULL,   -- per lens: hub cut-off, never global
  cluster TEXT NOT NULL,                 -- representative = min visible member of its live same_as component
  PRIMARY KEY (lens, entity_id)
) WITHOUT ROWID;
CREATE INDEX entity_state_by_cluster ON entity_state (lens, cluster);
CREATE TABLE alias_state (               -- from rastro:alias claims, with intervals: recycled logins (13-identity.md §5)
  lens INTEGER NOT NULL REFERENCES lens(id),
  alias TEXT NOT NULL, kind TEXT NOT NULL, entity_id TEXT NOT NULL REFERENCES entity(id),
  start TEXT NOT NULL, "end" TEXT, claim_id TEXT NOT NULL REFERENCES claim(id),
  PRIMARY KEY (lens, alias, kind, entity_id, start)
) WITHOUT ROWID;
CREATE INDEX alias_state_by_entity ON alias_state (lens, entity_id);

CREATE TABLE projection_generation (id INTEGER NOT NULL);   -- T4's counter, kept

-- LPG-shaped view for the owner's ad-hoc SQL and a future DuckPGQ / SQL-PGQ attach. Never used by scoped operations.
CREATE VIEW edge AS
  SELECT s.lens, c.subject AS from_entity, c.predicate, c.object_entity AS to_entity,
         c.id AS claim_id, s.status, s.revision
  FROM claim c JOIN claim_state s ON s.claim_id = c.id WHERE c.object_entity IS NOT NULL;
```

**Generation handling.** The generation holds group B and group C. A rebuild (T4 mechanism) builds them as TEMP tables and publishes with one DELETE + INSERT transaction. Group A and the logs (`runs`, `lens`, `projection_publication`) stay outside, so a rebuild never loses precious input or the run log (ADR 0009). The TEMP DDL comes from constants, not from T4's string replacement of `REFERENCES` (`T4:SS/src/lifecycle.rs:256-258`), which is fragile.

**Rule for every scoped query (testable):** read `claim`, `entity` and `justification` only joined to a lens table, or filtered by the lens's scopes. Never read `claim_state` or `entity_state` for a different lens. A test greps the storage crate's scoped SQL for violations.

**Write paths.**
| Path | Steps (one transaction per batch, with its checkpoint, as ADR 0009) |
|---|---|
| Incremental (`sync`) | 1. Validate and insert justifications. 2. Close the previous open justification in the same slot when the claim differs (`justification_by_slot`); the slot only appends because order within a pair is monotonic. 3. Touched set = claims plus their field groups plus premise dependents (walk `premise_by_claim`, bounded by strata). 4. For each lens covering the batch's scope or `*`: run `recompute(lens, touched)` and upsert C. 5. Write the checkpoint and `projection_publication` (kind `incremental`). |
| Rebuild | Lock, create TEMP shadows, load vocabularies, then per bound (source, extractor): a deterministic extractor re-runs its process (T4, `T4:CORE/src/runs/mod.rs:126-157`); a non-deterministic one replays its artifacts with re-validation and **no model call**. Replay intents in `created_at` order, run core inference by stratum, compute every lens, publish. |
| Re-extract | `rastro claims reextract <extractor> [--source]`: CLI only. Writes new artifacts, then runs the normal incremental path. |
| Intent (slices 4, 6, 7) | Validate the target revision against the surface's lens (renderers §5; on mismatch, an operation error naming the current revision), insert `intent`, then project as an incremental batch. |
| Lens registration | A writing open (CLI, or `rastro mcp` at launch, ADR 0006) with an unseen scope set inserts into `lens` and computes group C for it from group B. No re-extraction. |

**`recompute(lens L, claims K)`.** Rust; T4's `refresh` (`T4:SS/src/lifecycle.rs:45-201`) generalised and restricted to the touched set.
1. Visible justifications: provenance and confirmation with `scope ∈ L` (or L = `*`); inference with every premise live in L, bottom stratum first.
2. Per scalar field group G (`field_key`; confirmations join by subject and predicate), sweep the boundary times of the visible justifications. Provenance j is **effective at t** ⇔ it is slot-live at t ∧ ¬∃ a visible w ∈ G, slot-live at t, with claim(w) ≠ claim(j) and `w.source_updated_at > j.source_updated_at` (both non-null). This is T4's rule (`T4:SS/src/lifecycle.rs:96-102`). Non-scalar claims skip the outranking.
3. A claim is live at t if any justification is effective. Maximal live runs give `claim_interval`. On a boundary where claim X stops and claim Y starts in G, `closed_by_claim` = Y.
4. Links: `supersedes(Y→X)` ⇔ the latest interval of non-live X was closed by Y. This is acyclic: Y's latest interval starts no earlier than X's latest interval ends, and X's ends after it starts, so following superseded_by strictly increases the latest-interval start. `conflicts_with` ⇔ two live claims in G with neither outranking the other (tie or missing clock).
5. Status (precedence, first match):

| # | Condition in lens L | Status |
|---|---|---|
| 1 | A visible `reject` intent targets the claim and has not been undone | `retracted` (interval closed by the intent) |
| 2 | A live confirmation justification (with its anchor premise live, if any) | `confirmed` |
| 3 | An effective provenance or inference justification with `deterministic = 1` whose premises are all `observed` or `confirmed` | `observed` |
| 4 | Any other effective justification (non-deterministic rule, or a hypothesis premise) | `hypothesis` |
| 5 | None live, and the last justification to end was replaced or outranked, or ended because a premise was superseded | `superseded` |
| 6 | None live otherwise (a premise was retracted, or a confirmation was undone) | `retracted` |

   Default for a scalar confirmation: it stops being live when a visible provenance justification for a different claim in its group starts after the intent's time (Q4).
6. Revision v2 = `r_ + H("rastro/revision/v2", status, sorted links, Σ H(rkey) mod 2²⁵⁶)`. `rkey` = jkey minus `extractor_version`. The sum is a multiset hash, O(1) to update. A claim with 10k supports, such as a bot's type claim, no longer re-hashes every support on each new one, which T4 does in `T4:CORE/src/claims.rs:110`. A new domain tag is needed: T4 changed the encoding under the v1 tag, which main's own rule forbids (`CORE/src/claims.rs:74-76`, the comment "Changing any of this is a new version").
7. Upsert `claim_state`, `claim_interval`, `claim_link` and `justification_current`; for the subject and object: `adjacency`, `entity_state` (types, display alias, degrees, cluster) and `alias_state`.

---

## 4. Worked examples

Ids are real v1-encoding hashes (checked against the golden test `CORE/src/claims.rs:281-290`). Times are UTC on 2026-11-04. Two lenses are registered: `*` (id 1) and `personal` (id 2); in (a), `work` (id 3) too.

### (a) PR #12: open → closed → open, observed by two sources
PR `https://github.com/acme/app/pull/12` = `e_b48ba2010706ad991b6dd7c2ee773aee`. O = `code:state "open"` = `c_44348de72df043081fd94e9736a70388`. C = `"closed"` = `c_829707326a2394df7a8e982ad5783943`. Source A = `github-work` (scope `work`); B = `github-oss` (scope `personal`). The timeline follows 11-time-provenance.md's concrete example.

| Captured | Evidence | Slot | Justification inserted (base) | Slot-local close |
|---|---|---|---|---|
| 09:00 | ev101 A `/state` "open", updated 08:55 | sA | j1 O | |
| 09:02 | ev201 B "open", 08:55 | sB | j2 O | |
| 10:05 | ev102 A "closed", 10:00 | sA | j3 C | j1 `end_at` 10:05, `ended_by_evidence` ev102, `replaced` |
| 10:10 | ev202 B "closed", 10:00 | sB | j4 C | j2 → 10:10 by ev202 |
| 11:05 | ev203 B "open", 11:00 | sB | j5 O | j4 → 11:05 by ev203 |
| 11:10 | ev103 A "open", 11:00 | sA | j6 O | j3 → 11:10 by ev103 |

All six justifications share one `field_key` = H(PR, code:state, PR URL, `/state`). Lens state after 11:10:

| Lens | `claim_interval` O | `claim_interval` C | O state | C state | Links |
|---|---|---|---|---|---|
| `*` | [09:00 ev101, 10:05) closed by ev102 / C; [11:05 ev203, –) | [10:05 ev102, 11:05) closed by ev203 / O | observed, live j5, j6 (`live_count` 2), `known_since` 11:05 | superseded, `known_since` 10:05 | O supersedes C |
| `personal` | [09:02 ev201, 10:10) by ev202 / C; [11:05 ev203, –) | [10:10 ev202, 11:05) by ev203 / O | observed (j5) | superseded | O supersedes C |
| `work` | [09:00, 10:05) by ev102 / C; [11:10 ev103, –) | [10:05, 11:10) by ev103 / O | observed (j6) | superseded | O supersedes C |

- **10:05 in `*`:** j2 (O, updated 08:55) is still slot-live in sB, but j3 (C, 10:00) outranks it, so O closes and C opens: exactly one transition, recorded on both intervals.
- **11:05–11:10 in `work`:** C stays current, because the reopen was seen only by the personal source. The work view is correct for what work can see (D18: "a visible claim is never hidden by a hidden replacement").
- **No cycle.** C's latest interval was closed by O, so `superseded_by(C) = O`. O is live, so it has no `superseded_by`. The O→C→O history is read from `claim_interval.closed_by_claim`, not from links (11-time-provenance.md's cycle problem).
- **Revisions:** O's revision changes at 09:02 (a second support), 10:05, 11:05 and 11:10. The claim id never changes (S8, S9).
- **Tie variant:** if j2 and j3 had equal `source_updated_at`, or either were NULL, neither outranks; O and C are both observed with `conflicts_with` in both directions (S11).
- **Missing field (S10):** an observation without `/state` creates no justification, so nothing closes.

### (b) A GitHub account and a Slack account become one Person, then undo
Entities: GitHub account `github:account/583231` = `e_2bd7b50c426cf14d9beb347a3b737354` (source `kalios-api`, scope `personal`). Slack account `slack:account/T01ABC/U02XYZ` = `e_68476c311d1ffeebfc6e03e669833e21` (source `slack-acme`, scope `work`).

1. **Exact identifier, no merge.** The Slack connector reads `/profile/email` = `felippe@example.com` (structured, vouched through `users:read.email`; 13-identity.md §6). Deterministic rule `slack.held-by-email/1` submits `rastro:held_by(slack → person:email/felippe@example.com)`. That Person is `e_73a8a3bf97c7654088f6b9aa40055ef6`; the claim is `c_eab2099b91a98ffb76ed2e7498a070d6`, observed, with provenance j70 in scope work. Minting a Person from an exact identifier is knowledge §24, not a merge.
2. **Candidate, then Ask.** The slice-4 candidate generator notices login `fbisca` against display name "Felippe Bisca". Names only *raise* a candidate (knowledge §4). It becomes an Ask only when it matters (13-identity.md §5, criterion 3), say because a work commitment names the GitHub account.
3. **Answer.** The owner answers "same human" while working in work context:
```sql
INSERT INTO intent VALUES ('i_7c…','answer','2026-11-04T16:00:00Z','work','cli',NULL,'same');
INSERT INTO intent_target VALUES ('i_7c…',0,'c_2d2bdcf430cda0a71dc5dcaed52a23ee',NULL,
  'github:account/583231','code:Account','rastro:held_by','person:email/felippe@example.com','rastro:Person',NULL,NULL);
-- projection: claim row c_2d2b… (gh → Person) + justification j71 kind 'confirmation', scope 'work', intent_id 'i_7c…'
```
   - **Lens `*` and `work`:** `c_2d2b…` is confirmed. Person `e_73a8…` has two incoming `held_by` rows in `adjacency`, and `about(gh, expand: identity)` unions both accounts' pages.
   - **Lens `personal`:** j71 (work) and j70 (work) are invisible. The Person isn't visible at all (no `entity_state` row), so the personal view can't learn that a Slack identity exists (13-identity.md §8).
   - **Every existing claim id is unchanged**, for example `code:author(PR12 → gh)` = `c_22aebb8524200aaea47e65c3c123c155`. Nothing is re-keyed (overlay).
4. **Undo.** `rastro undo i_7c…` writes two precious rows in one transaction: `('i_9d…','undo',…,'work',…,undoes='i_7c…')` and `('i_9e…','reject',…,'work')`, whose target is the same statement (the cannot-link; 13-identity.md §4).
   - **Base:** j71 gets `end_at` = i_9d's time and `end_reason` 'undone'.
   - **Lens `*`/`work`:** `c_2d2b…` is retracted by precedence 1; its interval is closed by intent i_9e. The Person is back to one account; the gh cluster is itself.
   - **Afterwards:** the candidate generator sees a live reject on this statement and never asks again (levels §1). Rebuild replays i_7c, i_9d and i_9e and lands in the same state. Any Action run whose basis held `c_2d2b…@rev` is flagged (knowledge §10).
5. **When `same_as` appears instead:** if an address-book Person `person:contact/google/c123` lists both `felippe@example.com` and `fbisca@gmail.com`, a deterministic rule submits `rastro:same_as(min, max)` between the two email-minted Persons. The cluster becomes both, with the representative being the smaller id among *visible* members. Undo works the same way.

### (c) Slack hypothesis → owner confirmation → retraction carrying through
Evidence ev9001: `slack-acme`/work, location `slack:message/T01ABC/C09/1730800000.000100`, `/text` [0,35) "I'll send the deck to Ana by Friday", trust class `unstructured`. ev9002 is the structured author `/user` = U02XYZ.

1. **Extraction (slice 5).** `rastro-extractor-commitments` 0.1, rule `commitment-from-message/1` with `deterministic = false`, run 77. Model output → artifact `a_51…` (`artifact_input`: 9001, 9002). Its submissions form one candidate, grouped by an additive, optional `candidate` field on `claim.submit`. The anchor entity is `slack:message/…#/text[0,35)` = `e_31084dcf837d9f48d851bb43d92cd3bf` (C).

| Claim | Id | Justification | Premises |
|---|---|---|---|
| `rastro:type(C) = std:Commitment` (anchor) | `c_b11473b5f581bde0d3a105a96f62443f` | provenance ev9001 [0,35), artifact a_51, det 0 | – |
| `std:owes(slack acct → C)` | `c_a0c3…` | provenance ev9002 `/user` + ev9001 [0,35) | anchor |
| `std:description(C) "send the deck"` | `c_24b54876447b5f836b2b4ac7da2860c0` | ev9001 [5,18) "send the deck" | anchor |
| `std:due_expression(C) "by Friday"` | `c_16d8e04c0466c1a5cd9aadeedc7aa3aa` | ev9001 [26,35) "by Friday" | anchor |
| `std:counterparty_mention(C) "Ana"` | – | ev9001 [22,25) "Ana" | anchor |
| `std:owes(Person → C)` | `c_4007e5e31986bb0541630e32b8cba006` | **inference** `rastro.lift-to-person/1` (det) | `c_a0c3…` ∧ `held_by(slack → Person)` `c_eab2…` |
| `std:due_by(C) "2026-11-06"^^date` | `c_fe33dd2d3a982a1740aab15db50b3d62` | **inference** `rastro.resolve-due/1` (det; captured_at + owner timezone) | `c_16d8…` |

   - Before storing anything, the core byte-checks every quote: `[26,34)` quoting "by Friday" is rejected (15-llm-ontology.md §10). "Ana" alone never selects a Person (knowledge §4).
   - Lens `work`: every claim is a `hypothesis`. The extractor-side ones come from rule 4; the inferred ones because a premise is a hypothesis, even though their own rule is deterministic.
2. **Confirmation (slice 6 queue).** The owner ticks "confirm" on the commitment card, giving intent `i_c1`, kind confirm, scope work, with five targets at the revisions shown (anchor, `owes(acct)`, description, due_expression, `owes(Person)`). Each confirmation justification gets premise = anchor (Q6).
   - Status: all five `confirmed`. `due_by` becomes `observed`: its deterministic rule now has a confirmed premise (Q8).
   - A rule then runs Act "create tracker task" → Action run R1 with basis [`c_4007…@r`, `c_fe33…@r`]. That lives in the execution graph, outside the projection (plugins §8).
3. **Retraction.** It wasn't a promise; the owner was quoting a colleague. Intent `i_r1`, kind reject, scope work, targets the anchor `c_b114…@r`. One incremental batch:
   - The anchor is `retracted` (precedence 1).
   - The `premise_by_claim` walk on the anchor finds `owes(acct)`, description, due_expression and the mention: their provenance and confirmation justifications all have the anchor as premise, so none is live → `retracted` (precedence 6).
   - The walk continues to `owes(Person)`: premise `c_a0c3…` is no longer live → `retracted`. The same happens to `due_by` through `c_16d8…`.
   - Events "claim changed: retracted" list 7 claim ids. The execution graph matches R1's basis → R1 is flagged and an undo is proposed (delete the tracker task). A pending "remind Ana Thursday" proposal is withdrawn (knowledge §10).
   - `held_by(slack → Person)` (`c_eab2…`) was only a premise *of* a retracted claim, so it is untouched. Carry-through runs downstream only.
4. **Replay.**
   - `claims rebuild`: artifact a_51 is re-validated with no model call, intents replay, the state is identical (S16 extended).
   - `claims reextract` with extractor 0.2: a new artifact. If it emits the same statements, the ids are the same and i_r1 still applies, so nothing resurfaces. If it picks span [5,35) for the anchor, the anchor is a **different entity** and resurfaces as a new hypothesis. That risk is named in §6 (R8).

### (d) `about(<account>)` with one-hop expansion and `why(<claim>)` under `--scope personal`
`rastro mcp --scope personal`. At launch the writing open finds lens 2 (`["personal"]`); each call then uses the read-only path in one snapshot (ADR 0002, 0006). Plans were taken with `EXPLAIN QUERY PLAN` on the DDL above after `ANALYZE` (a 20k-row hub: 21 rows read, no sort).

```sql
-- 0. Lens of this reader. A missing row (DB older than the launch): compute the bounded neighbourhood in memory, flagged.
SELECT id FROM lens WHERE scopes = '["personal"]';                          -- → 2

-- 1. Resolve the argument (D15: entity id, canonical alias or canonical URL; never a bare login)
SELECT entity_id FROM alias_state
 WHERE lens = :lens AND alias = :arg AND kind IN ('rastro:canonical','code:url') AND "end" IS NULL
UNION
SELECT entity_id FROM entity_state WHERE lens = :lens AND entity_id = :arg;  -- PK seeks; no row ⇒ empty result + coverage (S22)

-- 2. One hop, D15 order, bounded. Plan: SEARCH adjacency USING PRIMARY KEY (lens=? AND entity_id=?), no temp B-tree
SELECT claim_id, direction, predicate, known_since FROM adjacency
 WHERE lens = :lens AND entity_id = :e AND live = 1            -- history:true drops "live = 1"
   AND (:since IS NULL OR known_since >= :since)
 ORDER BY known_since DESC, claim_id
 LIMIT :limit + 1;                                             -- the extra row sets truncated

-- 3. Expansion (expand: identity): Persons this account is held by, then their other accounts.
--    Plan: SEARCH adjacency USING COVERING INDEX adjacency_by_predicate (lens, entity_id, direction, predicate, live)
SELECT c.object_entity AS person FROM adjacency a JOIN claim c ON c.id = a.claim_id
 WHERE a.lens = :lens AND a.entity_id = :e AND a.direction = 'out'
   AND a.predicate = 'rastro:held_by' AND a.live = 1;
SELECT c.subject AS member FROM adjacency a JOIN claim c ON c.id = a.claim_id
 WHERE a.lens = :lens AND a.entity_id = :person AND a.direction = 'in'
   AND a.predicate = 'rastro:held_by' AND a.live = 1
 LIMIT 21;                                                      -- cluster-size cap; over it ⇒ truncated_reason "identity_cap"
--    then query 2 per member (same + same_as cluster via entity_state_by_cluster); k-way merge in Rust; top-k is exact
--    because each member page is already sorted. Group "equal modulo cluster" in Rust (A4).

-- 4. Hydrate the page (≤ 51 rows)
SELECT c.id, c.subject, c.predicate, c.object_entity, c.object_value, c.object_datatype,
       s.status, s.revision, s.live_count, s.known_since,
       es.display_alias AS subject_alias, eo.display_alias AS object_alias
  FROM json_each(:page) p
  JOIN claim c        ON c.id = p.value
  JOIN claim_state s  ON s.lens = :lens AND s.claim_id = c.id
  JOIN entity_state es ON es.lens = :lens AND es.entity_id = c.subject
  LEFT JOIN entity_state eo ON eo.lens = :lens AND eo.entity_id = c.object_entity;
```
For `about(github:account/583231)` in lens 2, step 2 returns the `code:author` rows of personal-source PRs (`c_22ae…` for PR #12 among them) and the `rastro:alias "fbisca"` claim (`c_120c6044de0b11b8045813a0b514827f`). Step 3 finds **no** `held_by` row: example (b)'s confirmation carries scope work. So the result is `expanded: []` with no hidden-member count. Coverage and extraction state are per visible pair, unchanged from D17.

```sql
-- why(c_44348de72df043081fd94e9736a70388) in lens 2
SELECT c.*, s.status, s.revision, s.valid_basis FROM claim c
  JOIN claim_state s ON s.lens = :lens AND s.claim_id = c.id WHERE c.id = :claim;   -- no row ⇒ operation error (S25)
SELECT start, "end", opened_by_evidence, opened_by_intent, closed_by_evidence, closed_by_intent, closed_by_claim
  FROM claim_interval WHERE lens = :lens AND claim_id = :claim ORDER BY start;
SELECT kind, to_claim FROM claim_link WHERE lens = :lens AND from_claim = :claim
UNION ALL
SELECT 'superseded_by', from_claim FROM claim_link WHERE lens = :lens AND to_claim = :claim AND kind = 'supersedes';
-- Supports, bounded. Plan: SEARCH justification USING INDEX justification_by_claim; sort of one claim's rows only
SELECT j.id, j.kind, (jc.justification_id IS NOT NULL) AS current, j.evidence_id, e.source, e.scope, e.location,
       j.field, j.start, j."end", e.start AS evidence_start, e.quote, e.captured_at, e.source_updated_at,
       e.trust_class, j.rule, j.extractor, j.extractor_version, j.intent_id, j.artifact_id,
       j.start_at, j.end_at, j.end_reason
  FROM justification j
  LEFT JOIN evidence e ON e.id = j.evidence_id
  LEFT JOIN justification_current jc ON jc.lens = :lens AND jc.justification_id = j.id
 WHERE j.claim_id = :claim
   AND (j.scope IN (SELECT value FROM json_each(:lens_scopes)) OR jc.justification_id IS NOT NULL)  -- inference rows are visible only if current
 ORDER BY current DESC, j.start_at DESC, j.id
 LIMIT :limit + 1;
-- why --premises: WITH RECURSIVE over justification_premise, depth ≤ strata, each level filtered by justification_current
```
Result in lens 2:
- Status `observed`. Intervals [09:02 ev201, 10:10 closed by ev202 / C) and [11:05 ev203, –). Link `supersedes c_8297…`.
- Supports: j5 (current) and j2 (ended, `replaced` by ev202), both from `github-oss`/personal. The excerpt is cut in Rust from `quote[start - evidence_start, end - evidence_start)` with clipping flags (surfaces §12).
- j1, j3, j4 and j6 (work) never appear. Neither does the work-lens fact that the PR stayed closed until 11:10.

---

## 5. Impact on T4/T5 and migration

**Order:** ADRs (below) → one migration `knowledge_v2` → **T4′** (T4 reworked onto it) → **T5′** (T5 on top) → T6–T8 as planned. T8's "scope filtering of every derived field" becomes "read the lens". Neither branch is merged as it stands.

### T4 `feat/claim-lifecycle` @4ac017f
| Location | Verdict | Change |
|---|---|---|
| `T4:MIG/202611061100_claim_lifecycle.sql` (whole) | Drop | Never applied anywhere that matters (owner DB lacks even T3's tables, `00-current-state.md` §4). Replaced by `knowledge_v2` |
| `T4:SS/src/lifecycle.rs:45-201` `refresh` | Keep as **test oracle**; rework for production | Becomes `naive_replay(lens)` in tests; production is `recompute(lens, touched)` |
| `…lifecycle.rs:61-63` (DELETE all, then replay all, on every batch) | Drop | O(all supports) per batch, so quadratic overall; global, not per scope |
| `…lifecycle.rs:22-31, 96-102, 119-131` | Keep | Field group without rule; strict newer comparable; a revived claim drops its old link (matches §3 step 4) |
| `…lifecycle.rs:203-231` readers | Rework | Read `claim_interval`, `claim_link` for the caller's lens |
| `…lifecycle.rs:233-284` TEMP rebuild | Keep | Extend the table list to groups B and C; replace `REFERENCES` string stripping (`:256-258`) with explicit TEMP DDL |
| `T4:SS/src/lib.rs:581` (`refresh` per `record_claims`) | Rework | Incremental touched-set recompute per lens |
| `T4:SS/src/lib.rs:601-612` `extraction_sources` with `Source::github(...)` | Rework | Build `Source` from the `sources` row (connector column) and `source_extractor` |
| `T4:CORE/src/claims.rs:110` `revision_with_links` under the v1 tag | Rework | `rastro/revision/v2`, multiset form (§3 step 6) |
| `T4:CORE/src/claims.rs:243` `scalar` from cardinality | Keep | Plus cardinality for meta-predicates: `rastro:type` one, `rastro:alias`/`held_by`/`same_as` many. Fixes the T4+T5 merge break at merged `claims.rs:288` |
| `T4:SDK:383-395` `Cardinality` defaulting to `Many` | Rework | Required field; `deny_unknown_fields` on vocabulary structs |
| `T4:CORE/src/claims.rs:254-277` `ClaimView` | Keep and extend | Five statuses; intervals carry opened and closed by; a support carries `kind` and `current` in the lens |
| `T4:CORE/src/runs/mod.rs:126-157` `rebuild_claims` | Keep | Add artifact replay (slice 5) and the `projection_publication` row |
| `T4:crates/rastro-cli/tests/claims.rs` | Keep | Retarget assertions to lens 1; add the §6 oracles |

### T5 `feat/github-extractor` @7a532bb
| Location | Verdict | Change |
|---|---|---|
| `crates/rastro-extractor-github/**` | Keep | Retag the 7 untagged unit tests (the converge FAIL) |
| `vocabulary.toml` | Keep | Already declares `cardinality` (:69, :83, :97); add `rastro:` dependencies |
| `T5:SDK:373-440` vocabulary format | Keep | Add `cardinality` (required) and `supertype`; `deny_unknown_fields` |
| `T5:CORE/src/claims.rs:16-21` `TYPE`/`ALIAS` = `"type"`/`"alias"` | Rework | `rastro:type`, `rastro:alias`, datatype `rastro:type`. Ids change; no stored data, so it costs nothing now |
| `T5:CORE/src/claims.rs:177-196, 257-270` | Keep | Meta-claim validation; the alias-must-be-quoted check (good) |
| `T5:SS/src/lib.rs:559, 578` `UPDATE entity SET type` | Drop | `entity_state.types` per lens, from type claims; two types = a visible conflict |
| main `SS/src/lib.rs:510-525` type clash → reject | Replace | **Type grounding** (A5) |
| `T5:SS/src/lib.rs:585` alias `INSERT OR IGNORE`, never removed | Drop | `alias_state` from alias claims, with intervals |
| `T5:SS/src/lib.rs:490-522` `location_context` | Keep and document | Add `context: true` to `BatchItem` (additive, protocol 1.x); record in an ADR 0008 amendment |
| `T5:CORE/src/host/extract.rs:270-275` type claims first | Keep | Grounding needs it |

### ADRs and decisions
| Doc | Action |
|---|---|
| ADR-0010 *Justifications and owner intents* (new) | `claim_support` → `justification` (three kinds, premises, OR/AND); `intent` precious; status precedence; carry-through |
| ADR-0011 *Scope lenses* (new; amends 0009) | Materialized per-scope-set state; registration by writing opens; read rule; fallback |
| ADR-0012 *Identity: overlay merges and Person hub* (new) | `rastro:` meta-predicates, Person minting, clusters per lens, undo as reject |
| ADR-0013 *Extraction artifacts and hypothesis acceptance* (new, slice 5; amends 0008) | Durable artifacts, rebuild ≠ re-extract, exact quote check, the hypothesis rule |
| ADR 0009 (amend) | Tables (§3), occurrences with transitions, revision v2, `projection_publication`, `valid_basis` |
| ADR 0008 (amend) | Location context; meta-predicates; type grounding; vocabulary format; `candidate` field |
| `knowledge.md` | Record the new decisions dated; **§12 amendment** if Q2 = artifacts; §26 "superseded_by" definition sharpened |
| `vocabulary.md` | Core meta-model predicates; cardinality required; datatype registry |
| `surfaces.md` | `expand`, `claims?`, `premises?` parameters (additive) |
| `GLOSSARY.md` | Only if Q3 changes "Merge". "Lens" and "slot" stay implementation names |

---

## 6. Risks and how we'd know we're wrong
| # | Choice | Falsifiable test or benchmark | Wrong if… → fallback |
|---|---|---|---|
| R1 | Lenses (materialized per scope set) | Synthetic 1M claims, 3 lenses, 30% two-scope claims: DB size, incremental batch p95 (100 evidence), rebuild time | Lens storage > 3× base, or batch p95 > 250 ms → recompute multi-scope lenses at read time over the bounded neighbourhood |
| R2 | Scope correctness | **Scope-isolation oracle:** for every scope S, every `about` and `why` (ids, statuses, revisions, intervals, links, aliases, types, counts) in lens {S} on the full DB equals the unscoped answer on a DB rebuilt from S's evidence and intents only. Content ids make the two comparable | Any diff (except FTS rank, an accepted limit) is a leak or a bug |
| R3 | Hand-rolled temporal logic | Property test: random sequences (A→B→A, ties, null clocks, two sources, version upgrades, intents) → incremental ≡ rebuild ≡ T4 naive replay; `superseded_by` acyclic; intervals of one claim disjoint | Any mismatch. If they keep recurring, evaluate expressing the rules in a Datalog crate (10-models.md "Leaning") |
| R4 | Hub fan-out | Bot with 100k authored PRs: `about(bot)` p95 < 10 ms; `about(PR, expand)` examined rows ≤ budget, with `truncated_reasons`; benchmark "bot is noise" and "bot is the answer" | p95 > 50 ms, or a budget overrun → adjust the adjacency key or caps |
| R5 | SQLite is enough (A7) | 12-engines.md's comparison gate: scoped about/why, search → two hops, hub, history at 100k/1M/10M claims, with ingestion running | Search → two hops p95 > 100 ms at 1M after index tuning → prototype a DuckPGQ sidecar (option E) |
| R6 | Overlay merges | Merge then undo: every pre-existing claim id and renderer target resolves unchanged; `about(expand: identity)` over 10 accounts × 5k claims p95 < 30 ms; grouping "equal modulo cluster" never collapses different statements | Ids change, or grouping errs → reconsider D (re-key) |
| R7 | Rebuild ≠ re-extract | Rebuild with the network off reproduces the projection byte for byte (status, revision, intervals); deleting an artifact changes only its own claims | A model call during rebuild, or any drift |
| R8 | Anchor identity (example c) | Re-extract the same corpus with a prompt change: share of candidates whose anchor id changes | > 5% → the anchor identifier becomes location + field + candidate ordinal |
| R9 | Revision v2 multiset hash | Fuzz: revision differs whenever the visible state differs (status, links, rkey set) over 10⁶ random states; the incremental update equals a recompute from scratch | Any collision or mismatch → sorted-hash encoding (O(n)) |
| R10 | TEMP-copy publication | 1M claims × 2 lenses: publish time, WAL peak, reader latency during publication | Publication > 60 s, or WAL > 2× the DB → generation-keyed tables with an active pointer |
| R11 | Fixed MCP operations (no query language) | Owner evaluation: 40–60 real questions (14-retrieval.md "Small owner-specific evaluation proposal"); count questions needing more than 3 tool calls or unanswerable | > 20% unanswerable with the fixed ops → design a bounded `path` or `expand` operation, still not a query language |
| R12 | Edge interop | Round trip: export TriG/JSON-LD → load into Oxigraph → SPARQL counts per predicate and status equal SQL counts on `edge` | Mismatch → serializer bug; if RDF export is never used in 3 months, drop it |

---

## 7. Questions for the owner (ranked)
| # | Question | Options (my leaning in bold) |
|---|---|---|
| Q1 | How do scoped reads stay exact (D18/S28) at O(limit)? | **a) Lenses, materialized per launched scope set**; b) compute at read time over the neighbourhood (slow on hubs, no extra storage); c) materialize the full scope only and accept leaks (contradicts D18) |
| Q2 | Replaying non-deterministic extraction | **a) Durable artifacts as a precious input (amend knowledge §12)**; b) call the pinned model again (replay not reproducible, ADR 0009 ids drift); c) a cache (if deleting it changes knowledge, it isn't a cache). **Q2b:** on a vocabulary *definition* change: **re-validate and mark `behind`**, or re-extract automatically? |
| Q3 | Accounts and Persons; the word "Merge" | **a) Person hub (`held_by`) + `same_as` between Persons; Merge glossary broadened to "…are one, or belong to one Person"**; b) the same model, Merge = `same_as` only, account linking called "link"; c) Person = cluster (13-identity.md P3) |
| Q4 | Confirmation semantics | i) Does a confirmation alone sustain a claim after its provenance is gone? **yes**/no. ii) Does a newer observation of a different value supersede a confirmed scalar? **yes**/no/Ask. iii) Does a reject block a claim that new evidence later supports? **yes, show a conflict flag**/no |
| Q5 | Which scope does an intent carry? | **a) the scope of the context it was given in**; b) every scope; c) the owner picks per intent |
| Q6 | Rejecting an anchor (a commitment) when its slots were confirmed | **a) Confirmations of a candidate are conditioned on its anchor, so everything is retracted**; b) independent: the slots stay confirmed with a flag |
| Q7 | Which trust classes may support a `hypothesis`? | **a) structured + unstructured; untrusted content only with an owner rule**; b) all; c) structured + unstructured only |
| Q8 | Status of an inference over observed or confirmed premises with a deterministic rule | **a) `observed`** (so it can back an Act); b) `hypothesis` until confirmed |
| Q9 | Meaning of "as of" (slice 7) | **a) re-derived under today's rules from evidence captured by t, plus the publication log**; b) also reproduce what was served (answer receipts, generation history) |
| Q10 | Expansion surface | **a) `about(expand: identity)` + `search(claims: true)` + a bounded BFS with lens-degree hub cut-off**; b) `about` one hop only; c) a generic `expand` or `path` operation |
| Q11 | Contextual qualifiers ("role at Acme", "price in 2025") | **a) model them as entities (Role, Membership)**; b) a qualifier table keyed by claim (not part of the id) |
| Q12 | Undo of a merge | **a) always records a cannot-link (reject)**; b) the owner chooses between "undo" and "they're different" |
| Q13 | Retired extractor versions | **a) delete their justifications once the new version catches up (incremental ≡ rebuild)**; b) keep them as history |
| Q14 | Ad-hoc graph queries for the owner | **a) the `edge` view through the CLI plus an RDF export for any SPARQL tool**; b) a `rastro query` SQL surface over lens views; c) none |

## Unverified
- No benchmark was run beyond the `EXPLAIN QUERY PLAN` and a 20k-row hub smoke test in `:memory:`. Every number in §6 is a proposed threshold, not a measurement.
- The cost of lens maintenance at 1M claims is an estimate (about 2–3× claim rows).
- The multiset-hash collision resistance against a deliberately crafted extractor (Wagner-style generalised birthday attack) was not analysed. Supports must be real, validated spans, which limits it, but this isn't proven.
- Slack `users:read.email` as a vouched identifier is taken from 13-identity.md §6, not re-checked.
- T4/T5 line numbers were read from the worktrees on 2026-11-05. I didn't build or run either branch.
