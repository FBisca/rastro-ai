# Retrieval over the knowledge graph

Research checked **2026-11-04**. Options for the owner; no new decisions. Primary papers, project documentation and repositories only. Numerical results below are authors' reports, not replications. Current documentation describes moving branches, not pinned installations.

## Rastro constraints and the gap to resolve

| Existing contract | Consequence for retrieval |
|---|---|
| Text search, then 1–2 graph links; summaries never replace evidence (`docs/decisions/knowledge.md:29`) | A community summary may route a query; the returned basis must still reach retained evidence and spans. A fluent summary is insufficient. |
| Coverage distinguishes found/empty/couldn't look/never looked; stale means unknown (`docs/decisions/knowledge.md:25`, `docs/decisions/knowledge.md:36`) | Report source coverage independently of search matches, graph completeness and retrieval truncation. |
| D15: `about` is one hop, known-time newest first then claim id; default 20/max 50 (`docs/plans/2026-11-04-claims-and-about.plan.md:49`) | Do not silently replace its ordering with relevance/PPR or turn it into two-hop search. Expansion needs a separately agreed contract. |
| D16–D18: bounded `why`, coverage and extraction state, visible-support-derived metadata (`docs/plans/2026-11-04-claims-and-about.plan.md:50`) | Retrieval must preserve claim → support → evidence/span, versions, status and clipping flags; scope filtering precedes ranking/limits and metadata. |
| Local stdio MCP calls the library through read-only per-call readers (`docs/adr/0006-mcp-server-over-stdio.md:12`); projection is rebuildable (`docs/adr/0009-claims-projection.md:8`) | No query-time extraction, persistent graph mutation or second authoritative store merely to answer a read. |
| FTS rank statistics currently span scopes (`docs/decisions/surfaces.md:51`) | Existing accepted leakage is not evidence that globally computed graph degrees, PPR, summaries or support counts are safe. |

The plan explicitly leaves traversal/index/store questions open (`docs/plans/2026-11-04-claims-and-about.plan.md:140`). Its recorded API is not evidence that the paused implementation already supplies those operations.

## What graph retrieval actually retrieves

| Method | Retrieved unit / algorithm | Measured comparison | Failure modes / applicability to Rastro |
|---|---|---|---|
| **Microsoft GraphRAG** [S1–S3] | Global: hierarchical community reports, map/reduce answers. Local: embedded entity seeds → relationships, text units, reports and optional covariates. DRIFT: reports seed follow-up questions answered through local search. | Paper v2, global questions: C0 vs vector baseline comprehensiveness preference **71.92%** (podcasts), **71.76%** (news); diversity **76.56% / 62.08%**. These include half credit for ties; not answer accuracy. C0 vs text-only map/reduce comprehensiveness **50.24% / 55.52%**. | Graph benefit is smaller against a strong text summarization baseline. Extraction/identity errors and recursive compression can erase exceptions and provenance. Global summaries alone violate Rastro §15; local text-unit mappings are the transferable part. DRIFT adds model calls and variable exploration. |
| **LightRAG** [S4–S5] | Low-level entity keywords and high-level relationship/theme keywords retrieve entity/relationship descriptions and associated original text; graph/vector combination. Current repo also provides citations. | Original paper v1, LLM-judged overall preference vs NaiveRAG: **66.70 / 65.24 / 82.54 / 62.40%** on Agriculture/CS/Legal/Mix. | Preference for broad answers does not establish exact claim recall. Incorrect extracted links/descriptions propagate; keyword generation can miss identifiers. Citation capability is not proof of claim-level entailment. |
| **HippoRAG 2** [S6] | Query-to-triple retrieval and LLM triple filtering seed personalized PageRank (PPR) over phrase/passage graph; returns ranked **passages**, with dense fallback. | Same Llama-3.3-70B reader and NV-Embed-v2 retriever: mean QA F1 **59.8 vs 57.0** dense; MuSiQue **48.6 vs 45.7**, 2Wiki **71.0 vs 61.5**. | Selected low-recall failures expose both bad triple filtering and bad graph ranking; filtering can remove all relevant seeds. PPR ranks association, not justification. It is not intrinsically a two-hop limit. Preserve actual evidence independently of graph scores. |
| **KG²RAG** [S7] | Semantic seed chunks → KG-guided chunk expansion → query-weighted maximum spanning trees per component → reranked, ordered source chunks. | Hotpot-Full answer F1 **0.631 vs 0.587** semantic + rerank; retrieval F1 **0.310 vs 0.306**. Distractor answer F1 **0.663 vs 0.652**. | Gains vary strongly by setting/metric. A wrong or absent bridge limits expansion; spanning-tree filtering can discard useful alternate links. Closer analogue to Rastro's seed-and-expand than summary-only answering, but its extracted triples still need validation. |
| **Think-on-Graph / ToG-2** [S8–S9] | Agent iteratively chooses relations/entities with beam search and returns reasoning paths; ToG-2 alternates KG traversal and linked document retrieval. | ToG reports SOTA on **6/9** datasets; ToG-2 **6/7** with GPT-3.5. These are dataset counts, not a fixed-budget gain over Rastro-like text RAG. | Wrong entity/beam pruning blocks the answer; missing KG edges force stopping or unsupported completion. Iteration multiplies calls; a returned path shows connectivity, not that a conclusion follows. |
| **ToG-3 / agentic exploration** [S10] | Query-time evolution of a chunk–triplet–community subgraph, retrieval, answer drafting and sufficiency reflection. | Paper reports outperforming its deep/broad reasoning baselines; no independently verified comparable scalar gain here. | Useful exploration pattern, but query-time graph construction conflicts with Rastro's read-only materialized projection if persisted. A client could explore existing claims with bounded reads without adopting construction. |
| **Template-based KG-RAG** [S11] | Neo4j's VectorCypher/HybridCypher retrievers use vector/hybrid seeds followed by application-supplied traversal; Text2Cypher generates a query. | Documentation demonstrates mechanisms, not a universal measured advantage. | Separates graph storage from retrieval policy. Templates bound the supported questions; generated queries introduce schema and permission failures. |

**Evidence against a universal winner:** the 2026 scaling study [S12] holds questions/reader fixed over 28 nested corpus tiers. BM25 leads from mid-scale; graph builders encounter cost/scaling limits. Raw-file agency costs 39× query tokens at the smallest tier. At full scale, **Agent+BM25 69.4 vs native BM25 54.8 vs raw-file agent 36.9** on the same 150 questions: agency and indexing interact. This does not establish a graph-expansion threshold for one owner's graph.

The earlier repo research's “−13.4% on NQ” (`docs/research/knowledge-graphs-2026-11.md:17`) appears in GraphRAG-Bench's introduction as a citation to another study, not its own experiment [S13]. Do not treat it as a reproduced local result. Two different papers use “GraphRAG-Bench”; [S13] is arXiv **2506.05690**, distinct from **2506.02404** [S14].

## Agent memory: APIs and source trails

“Exposes a source id” and “proves a claim from an immutable span” are different guarantees. No integration was executed in this research.

| System | Query surface exposed to agents/applications | Source trail and gap for Rastro |
|---|---|---|
| **Zep / Graphiti** [S15–S17] | Graphiti `search(query)` combines semantic/BM25 with RRF; focal-node distance reranking available. `search_` configures edge/node/episode/community retrieval and rerankers. Zep v4 separately exposes edge/node/episode searches. MCP: `search_nodes`, `search_memory_facts`, `get_entity_edge`, `get_episodes`, `get_episode_entities`; supports group and temporal filters. | Episode provenance and edge UUIDs allow follow-up to input episodes. MCP's provenance operation is a useful `why` analogue; episode references alone do not guarantee exact field/byte spans. Exposing mutation tools alongside reads is a separate permission choice. |
| **Mem0 graph memory** [S18–S19] | Current Platform `client.search(query=..., filters={"user_id": ...})`: entity-connected memories receive a boost combined with vector/BM25 into one score. OSS v3 removes external graph-store integration. | Current `relations` is unpopulated; graph connectivity is folded into ranking, not returned as traversable paths. A memory hit/metadata needs an application-maintained link to original evidence. Old `enable_graph` examples do not describe the current API. |
| **Cognee** [S20–S22] | Current `recall(query_text, query_type?, datasets/dataset_ids?, only_context?, include_references?)`; legacy `search` still documented. Chunk, graph, temporal and agentic modes; direct Cypher opt-in. `only_context` skips final completion/session QA write. | Chunk results expose data/chunk ids; references can append an `Evidence:` block and populate `metadata.evidence` for supported completion modes. Graph citations require an edge-evidence sidecar migration; hybrid mode has narrower metadata. Document ids are not Rastro spans. Docs disagree on reference defaults and scope handling: see Unverified. |
| **Letta** [S23–S24] | V1 documented `archival_memory_search` retrieves passages; API returns ids, content, timestamps and tags. Conversation history is separately searchable; core memory blocks sit in context. Current SDK has a separate memory surface. | Passage/message ids can support follow-up; generated memory text does not inherently retain the message/evidence that justified it. Pin SDK generation: do not mistake V1 archival APIs for the current SDK contract. No graph traversal required. |
| **LangGraph / LangMem** [S25–S26] | Store JSON items by namespace/key; semantic query and metadata filters. LangMem `search_memory(query, limit=10, offset=0, filter?)`; manage tool creates/updates/deletes configured memory schemas. | Namespace, key, timestamps and score identify a stored item. Evidence ids/spans and immutable history must be part of the application's schema. LangGraph execution edges are not a knowledge graph. Namespace supplied by trusted runtime, rather than agent choice, can express a boundary. |
| **MCP reference memory server** [S27] | `search_nodes(query)` searches names/types/observations; `open_nodes(names)` returns selected nodes and their mutual relations; `read_graph` returns everything. Also mutation operations. | String observations and named relations, with no required immutable source/span/coverage model. A useful minimal operation example, not equivalent to Rastro's evidence model. Whole-graph reads have unbounded disclosure/context implications. |

## Agent query surface: options, not a decision

| Surface | Strengths | Costs / enforcement needed |
|---|---|---|
| **Fixed `search`, `about`, `why`** | Small schema/context overhead; reproducible bounded responses; core owns status, scope, citations and coverage. Matches recorded contracts. | Several tool round trips; cannot express arbitrary joins/counts/paths. A cap can hide a decisive claim unless surfaced. |
| **Fixed operations + bounded `expand` / path template** | Agent chooses next entity/predicate/direction; server owns traversal limits and evidence envelope. Covers many entity-centric tasks without query-language generation. | New operation/semantics require owner agreement. Need per-hop/global budgets, continuation and explicit predicate semantics. Raw tool calls still cost tokens/latency. |
| **Cypher or GQL** | Expressive property-graph patterns, aggregation and paths. Neo4j MCP demonstrates schema/read-query tools [S28]. | Model needs schema/vocabulary and query repair; syntax/dialect/schema drift. GQL is not an interchangeable alias for every Cypher dialect. Read-only transactions/credentials, procedure restrictions, resource budgets and server-side scope enforcement remain necessary. |
| **SPARQL** | Explicit RDF graph patterns and property paths [S29]; useful if RDF interoperability is chosen. | `SERVICE`/dataset selection, inference and variable-length paths broaden work/disclosure. Endpoint property paths do not automatically return an evidence-bearing ordered edge chain. Restrict federation and require provenance joins. |
| **SQL** | Direct joins against claim/support/evidence tables; familiar query planning; precise reports. | Leaks physical schema; omission of scope/status/time/support joins is easy. SELECT-only is insufficient if functions/extensions can access other resources. A protected logical view is safer than arbitrary access to underlying tables. |

**Design analysis:** fixed operations narrow failure modes; query languages expand expressiveness. Neither MCP nor a tool's read-only annotation provides row-level security or guarantees bounded work. Query-language access would require a separate accepted surface/security contract, not a new argument to current tools.

Possible safeguards for either route:

- Resolve launch grants before execution; construct the visible graph from visible supports. Apply visibility at **every intermediate node/edge**, including ranking, counts, aliases, conflict links and summary inputs. A hidden bridge must not connect visible endpoints.
- Bound evaluated work, wall time and output bytes/tokens independently. A final `LIMIT 20` can follow an enormous path join or aggregation. Disallow graph-wide dumps and unrestricted procedures/federation in an agent-facing query language.
- Return claim/support ids and source spans through a common envelope; arbitrary scalar aggregates also need their query/filter/snapshot and basis. Do not let free-form queries bypass coverage.
- Determinism: fixed snapshot/generation, stable id tie-breaks, explicit scoring/model/index versions. SQL/Cypher ordering is not deterministic without an order; LLM query planning remains variable even with a deterministic executor.
- Distinguish **source coverage**, **extraction state**, **retrieval completeness** (hops/predicate filters/caps/timeouts), and **quote clipping**. “No connecting path within two hops” is not “no relationship exists.” Never expose hidden totals to explain truncation.

## Traversal and ranking experiments

The following are candidate policies to measure, not product defaults. Generic graph mechanics do not establish usefulness for the owner.

1. **Seed:** existing FTS over retained evidence; resolve each hit through support links to claims/entities. Keep text hits even when extraction is absent/behind; do not discard uncatalogued evidence. Exact aliases can separately seed `about` under its current contract.
2. **Expand:** choose semantic entity-to-entity claim edges in both required directions; retrieve literal-valued claims on selected entities as attributes. Count entity hops explicitly; following a support to its evidence is citation lookup, not another semantic hop. Type/alias/provenance links should not create unconstrained graph shortcuts.
3. **Filter before budget/rank:** visible supports, current/history policy, time, predicate/direction. Do not use globally stored status or degree when they depend on hidden supports. Preserve conflicts; ranking cannot resolve them.
4. **Bound:** experimental sweep: 5/10 seeds × 10/25 eligible edges per frontier entity × 1/2 hops, plus a global examined-edge and output-token ceiling. Record cap reasons, traversed hops and omitted frontier categories; avoid claiming exhaustive totals unless actually computed visibly. Per-parent caps control work earlier than a final result limit.
5. **Rank/deduplicate:** retain seed relevance and path explanation, deduplicate by claim/evidence id, then allocate the context budget. Cap excerpts and supports separately; allow `why` to obtain the full basis on demand.

| Signal | Potential gain | Failure / control |
|---|---|---|
| Query relevance + hop decay | Nearby bridge claims survive without flooding context | Two weak links are not an inference. Return ordered claim ids, predicates and directions. |
| PPR seeded by hits [S6] | Several routes reinforce related passages; transition normalization divides a node's mass over neighbors | Hubs remain influential; full PPR can exceed two hops. Test on a bounded visible subgraph with fixed restart/iteration settings; disclose approximation. |
| Recency | Finds current changes/commitments | Known time is learning time, not valid time; an unchanged old claim can remain relevant. Use query intent and freshness, not universal age deletion. |
| Support count | Corroboration can rank above one-off claims | Duplicated/syndicated sources are not independent; repeated observations should not inflate support. Count visible distinct provenance and cap/log-scale the feature. |
| Trust class / source authority / status | Distinguishes extraction eligibility and usable basis | Trust class is not an ordinal truth score; unstructured evidence may be highly relevant. Source authority is predicate-specific; ranking must not promote hypothesis to observed/confirmed or authorize an Act. |
| Predicate/type diversity and hub penalty | Limits domination by repositories, bots or generic entities | Penalizing a hub may hide the requested answer. Keep direct exact matches and allow focused follow-up; do not drop bot evidence. |

**10k-claim bot example:** a PR seed's `code:author` reaches a bot; unconstrained incoming traversal then retrieves thousands of unrelated items. Options: stop expansion at account/repository hubs unless explicitly targeted; partition neighbor budgets by predicate/time/source; query-select neighbors before taking a cap. If the question is explicitly about the bot, paginate/focus instead of suppressing it. Store/return evidence regardless of expansion policy. Benchmark both “bot is noise” and “bot is the answer.”

**Path queries:** shortest paths explain connectivity; bounded typed paths answer domain questions more precisely. Return every edge's claim id/status and retrievable support, not only endpoint names. Cycle detection and depth limits matter; all-path enumeration grows explosively. Bidirectional search helps endpoint-to-endpoint reachability but still needs budgets and visibility. SPARQL property paths and Cypher shortest-path selectors have different result semantics [S29–S30]. A justification dependency walk for `why` is separate from an entity-neighborhood walk.

## Index design: relational versus native graph

Illustrative logical keys, **not a migration**. The physical schema and scoped-status computation must be checked before implementing indexes. Generation prefixes preserve the projection model; use immutable canonical ids, not display aliases.

| Access pattern | Relational index / execution option | Trade-off |
|---|---|---|
| Outgoing / incoming one-hop claims | `(generation, subject_id, predicate, claim_id)` and `(generation, object_entity_id, predicate, claim_id)`; entity-object index can exclude literal objects | Both directions needed. Two selective scans plus deduplication avoid relying on one side of an `OR`; verify planner [S31]. |
| D15 known-time order, no predicate filter | Endpoint-leading keys with relevant known-time sort column and claim-id tie-break, **if that visible time can be safely materialized** | Putting unconstrained predicate ahead of time cannot satisfy global time order. Global stored time/status is insufficient for scoped answers; compute visible support state then sort when needed. |
| Visible supports / `why` | `(generation, claim_id, evidence_id)` plus support lookup from evidence; evidence/source/scope indexes for visibility semi-joins | Supports are many-to-one: filtering claims after limiting loses visible results. Avoid denormalized global support counts/status as authorization predicates. |
| Exact entity / alias resolution | Unique canonical-identifier lookup; alias → entity lookup followed by visible-support checks | Alias existence and display data can themselves leak a hidden scope. |
| Two hops | Bounded repeated adjacency joins or application BFS with explicit frontier; recursive CTE for variable depth [S32] | CTE convenience does not guarantee cost bounds. Deduplicate visited entity ids; `UNION` over `(id, depth, path)` does not by itself prevent cycles. SQLite recursive terms restrict aggregates/windows. |
| Covering indexes | Append narrowly selected neighbor fields when `EXPLAIN QUERY PLAN` shows avoided table lookups [S31] | Larger indexes increase ingestion/rebuild work; do not copy quotes/large literals into adjacency indexes. Scope joins may prevent a fully covering read. |
| Closure / reachability table | `(ancestor, descendant, depth, generation)` for repeated stable reachability queries | Potential O(V²) pairs; costly changes/retractions, time and scope variants. Reachability pairs alone lose path provenance. Benefit is weak for bounded 1–2-hop reads unless measured. |
| Native graph store | Indexed seeds followed by native adjacency/path operators; graph query language and algorithm ecosystem | Can simplify deep/path-heavy workloads; not automatically faster for selective one-hop joins. Still needs text/vector indexes, citations, visible-support status and budgets; hub fan-out remains. A separate engine adds snapshot/rebuild coordination. |

**Comparative measurement:** same evidence, claims, visibility rules and output contract; measure adjacency candidates examined, p50/p95 latency, memory, index size, ingestion/rebuild time and failure recovery at real scale plus synthetic hubs. A native engine becomes an option when this workload demonstrates an unmet need; graph-shaped data alone is insufficient evidence.

### Text + vector + graph fusion

- Separate candidate channels: BM25 exact terms/identifiers; optional vector paraphrases; bounded graph expansion for missing bridges. Normalize them to the **same candidate identity** (claim or evidence); node scores cannot be merged directly with chunk scores without a mapping.
- RRF [S33]: `score(x) = sum_i w_i / (k + rank_i(x))` for ranked lists containing x. No common BM25/cosine/PPR score scale needed. Choose weights/k on a development set, keep id tie-breaks and log component ranks. RRF does not fix a missing seed, bad edge or permission error.
- Filter each channel before selection; an approximate vector index may need iterative overfetch to fill an authorized top-k. Exhausted budget means incomplete, not empty. Global embedding/PPR/summary computation can introduce additional scope leakage.
- Preserve some direct text hits so expansion cannot consume the entire context. Optional reranking can improve precision but adds latency/model variance. Rebuild embeddings with model/version metadata; they remain disposable. No vector dependency is implied by this research.

## Evaluation for the owner

| Benchmark | Useful coverage | Missing for Rastro |
|---|---|---|
| LongMemEval [S34] | 500 questions: extraction, multi-session reasoning, updates, temporal reasoning, abstention; repo records cleaned histories in 2025 | Does not test Rastro scopes, source outages, immutable spans or owner usefulness. Pin dataset revision. |
| LoCoMo [S35] | Long multi-session dialogue QA/event summarization; generated conversations human-verified/edited; ~300 turns/9k tokens on average | Synthetic personas/event histories differ from work sources; vendor scores depend on chosen tasks, reader and judging. |
| LongMemEval-V2 [S36] | 2026 context-gathering benchmark: 451 questions on web-agent state/workflows/gotchas/premise awareness | Relevant to experienced agents, less directly to the owner's people/commitments. Do not conflate with original LongMemEval scores. |
| GraphRAG-Bench [S13–S14] | Distinguish graph construction, retrieval and final answer quality; include reasoning/synthesis | Different datasets share a name. Neither is an owner acceptance test. |

**Small owner-specific evaluation proposal (owner chooses questions/thresholds):**

- Freeze a local, consented evidence snapshot and projection. Collect **40–60 real questions** from morning define, commitments and project changes; preserve dates/scopes. Include direct lookup, alias/paraphrase, genuine two-link bridges, temporal change/conflict, and unknown/failed/stale coverage. Label only retained information; pointer-only ingestion cannot reconstruct omitted original text.
- Annotate acceptable answers **and minimal sufficient evidence sets**: claim/support/evidence ids and spans, valid/known-time expectations, allowed scopes and required abstention. Some questions have multiple sufficient sets. Keep raw-evidence answerability separate from whether extraction supplied a graph route.
- Add adversarial fixtures: 10k-claim bot, hidden bridge, hidden-only alias/replacement, duplicate supports, retracted/superseded basis, extraction behind, incomplete source pages, UTF-8 excerpt boundary. Future lifecycle cases are evaluation fixtures, not claims of current implementation support.
- Paired arms: **FTS alone**, FTS + one hop, FTS + two hops; optional vector alone / text+vector RRF / PPR arm. Fix snapshot, reader/prompt and context-token budget; also compare latency/cost budgets. A second control spends the same extra tokens on more text candidates rather than graph expansion.
- Retrieval metrics: recall of sufficient evidence sets at fixed tokens; added relevant bridge recall; precision/noise; nDCG; path validity and citation resolvability. Answer metrics: correctness, citation entailment/completeness, stale/conflict handling, correct abstention and coverage. Scope leaks and invented spans are failures independent of mean QA score.
- Measure cost of ingest/extraction/indexing separately from per-query tokens, tool rounds and p95 latency. Owner outcome: fewer source rechecks/corrections, missed commitments and duplicates; log task usefulness, not merely answer length.
- Split development/held-out questions by entity/time to reduce tuning leakage; never build the graph from test answers. Report per-category paired wins/losses and uncertainty, with owner review of changed answers. An LLM judge may assist; sampled evidence checks and the owner's judgment remain necessary.

## Choices left to the owner

- Preserve fixed APIs alone, add bounded expansion/path operations, or provide a restricted query-language surface?
- Which predicates/hubs may expansion cross, and how should incomplete retrieval be represented?
- Which query categories justify optional vector/PPR/summary routing, after equal-budget evaluation?
- Which latency/recall/provenance result would justify another store? No engine or ranking policy is selected here.

## Unverified

- No Rastro retrieval implementation, local latency measurement, benchmark rerun or current deployment inspection was performed. Proposed limits/indexes are hypotheses to test.
- Cross-paper numbers are not a leaderboard: corpora, readers, token budgets, metrics and implementation dates differ. HippoRAG 2's abstract “7%” is not interchangeable with its table's +2.8 mean F1 points. Current LightRAG should not inherit old reproduced failures without rerunning it.
- The underlying study behind GraphRAG-Bench's cited −13.4% NQ result was not independently inspected; no such quantitative claim is used to select an approach here.
- ToG-3's comparable fixed-budget gain and performance of all these methods on the owner's graph remain unverified.
- Cognee docs conflict: Search Basics lists both true and false defaults for `include_references`; its backend-access-control section says dataset filters are ignored when disabled, while Recall describes exclusive dataset scoping. Set references explicitly and verify pinned implementation/permission mode before relying on either behavior [S20–S22].
- Letta V1 search APIs were checked; equivalent current-SDK retrieval/citation semantics were not fully established. Graphiti episode provenance and Mem0 metadata were not tested for exact original-span retrieval. No memory system above was verified to implement Rastro's complete coverage contract.
- Live docs/main branches can drift; versions/release claims from earlier repo research were not re-audited. Checked date is not a release pin.

## Primary sources

Every entry checked **2026-11-04**. Publication/version dates shown where material; “docs/repo” means the current page, not a verified installed version.

| ID | Source / URL | Publication or scope |
|---|---|---|
| S1 | [Microsoft GraphRAG paper](https://arxiv.org/html/2404.16130v2) | v2, 2025-02-19; methods and Appendix G |
| S2 | [GraphRAG local search](https://microsoft.github.io/graphrag/query/local_search/) | docs |
| S3 | [GraphRAG DRIFT](https://microsoft.github.io/graphrag/query/drift_search/) | docs |
| S4 | [LightRAG paper](https://arxiv.org/html/2410.05779v1) | v1, 2024-10-08; Table 1 |
| S5 | [LightRAG](https://github.com/HKUDS/LightRAG) | official repo; current citations capability |
| S6 | [HippoRAG 2](https://arxiv.org/html/2502.14802v2) | v2, 2025-06-19; Tables 2–3, Appendix E |
| S7 | [KG²RAG](https://arxiv.org/html/2502.06864v1) | 2025-02; Tables 1–2 |
| S8 | [Think-on-Graph](https://arxiv.org/abs/2307.07697) | 2023-07 |
| S9 | [Think-on-Graph 2.0](https://arxiv.org/abs/2407.10805) | 2024-07 |
| S10 | [Think-on-Graph 3.0](https://arxiv.org/abs/2509.21710) | 2025-09 |
| S11 | [Neo4j GraphRAG retrieval guide](https://neo4j.com/docs/neo4j-graphrag-python/current/user_guide_rag.html) | docs |
| S12 | [Which RAG Paradigm Wins at Scale?](https://arxiv.org/abs/2607.26497) | 2026-07-29 |
| S13 | [When to Use Graphs in RAG](https://arxiv.org/html/2506.05690) | GraphRAG-Bench; 2025-06 |
| S14 | [Challenging Domain-Specific Reasoning](https://arxiv.org/abs/2506.02404) | different GraphRAG-Bench; 2025-06 |
| S15 | [Graphiti search](https://help.getzep.com/graphiti/working-with-data/searching) | docs |
| S16 | [Graphiti MCP operations](https://github.com/getzep/graphiti/blob/main/mcp_server/README.md) | official repo |
| S17 | [Zep v4 search](https://help.getzep.com/v4/searching-the-graph) | docs |
| S18 | [Mem0 graph memory](https://docs.mem0.ai/platform/features/graph-memory) | Platform docs |
| S19 | [Mem0 OSS migration](https://docs.mem0.ai/migration/oss-v2-to-v3) | v2→v3 docs |
| S20 | [Cognee recall](https://docs.cognee.ai/core-concepts/main-operations/recall) | current API, evidence and Cypher restrictions |
| S21 | [Cognee legacy search](https://docs.cognee.ai/core-concepts/main-operations/legacy-operations/search) | mode definitions and caps |
| S22 | [Cognee Search Basics](https://docs.cognee.ai/guides/search-basics) | citations, permissions; inconsistencies noted |
| S23 | [Letta archival search API](https://docs.letta.com/api/resources/agents) | API reference |
| S24 | [Letta archival memory](https://docs.letta.com/v1-sdk/memory/archival-memory) | V1 SDK docs |
| S25 | [LangGraph stores](https://docs.langchain.com/oss/python/langgraph/stores) | docs |
| S26 | [LangMem memory tools](https://langchain-ai.github.io/langmem/reference/tools/) | API reference |
| S27 | [MCP reference memory server](https://github.com/modelcontextprotocol/servers/tree/main/src/memory) | official repo |
| S28 | [Neo4j MCP](https://github.com/neo4j/mcp/blob/main/README.md) | official repo; query tools/read-only configuration |
| S29 | [SPARQL 1.1 query language](https://www.w3.org/TR/sparql11-query/) | W3C Recommendation, 2013-03-21; property paths/federation |
| S30 | [Cypher shortest paths](https://neo4j.com/docs/cypher-manual/current/patterns/shortest-paths/) | docs |
| S31 | [SQLite query planner](https://sqlite.org/queryplanner.html) | composite/covering/OR indexes and sort order |
| S32 | [SQLite WITH](https://sqlite.org/lang_with.html) | graph CTEs and recursion limits |
| S33 | [Reciprocal Rank Fusion](https://research.google/pubs/reciprocal-rank-fusion-outperforms-condorcet-and-individual-rank-learning-methods/) | Cormack, Clarke, Büttcher; SIGIR 2009 |
| S34 | [LongMemEval](https://github.com/xiaowu0162/LongMemEval) | official repo; ICLR 2025; history update notice |
| S35 | [LoCoMo](https://snap-research.github.io/locomo/) | authors' project page, 2024 |
| S36 | [LongMemEval-V2](https://arxiv.org/abs/2605.12493) | 2026-05-12 |
