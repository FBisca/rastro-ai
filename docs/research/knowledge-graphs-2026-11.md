# Knowledge graphs for the fleet: research (2026-11-04)

Sources: web research subagent (primary sources, dated), Codex independent review, read of mycelium (https://github.com/mycelium-ai-labs/mycelium). Companion: `docs/HANDOFF.md`.

## Bottom line
In 2026 the evidence keeps showing graph *retrieval* (GraphRAG family) losing to BM25/plain files on general Q&A. Graphs still pay off for **typed, time-scoped facts with provenance**, maintained by code and a human: people, commitments, decisions. Recommendation: grow the SQLite brain into a small typed graph. Don't adopt a GraphRAG framework or a graph DB.

## Concepts
- **Property graph**: nodes + typed edges, both carry properties (Cypher/GQL). Used by almost all agent tooling. **RDF** (triples, global IRIs, OWL, SPARQL) shows up only as optional ontology input (Cognee).
- **Schema/ontology**: the allowed node/edge types and which pairs may connect. Graphiti: Pydantic `entity_types`, `edge_types`, `edge_type_map`; new types need re-ingest. https://help.getzep.com/graphiti/core-concepts/custom-entity-and-edge-types
- **Entity resolution**: deciding "Ana" = "A. Lima" = ana@…; called the real bottleneck, harder than extraction. https://atlan.com/know/ai-agent/knowledge-graph/knowledge-graph-construction-for-ai/
- **Bitemporal edges**: valid_from/valid_to (true in the world) + recorded_at/invalidated_at (when we learned/unlearned). Contradictions invalidate old edges, never delete. Zep paper arXiv 2501.13956 (2025-01).
- **Communities/summaries**: GraphRAG clusters entities (Leiden) and pre-summarizes for "global" questions. arXiv 2404.16130.

## Graph retrieval (GraphRAG and co.)
- MS GraphRAG v3.3.0 (2026-10-08); LazyGraphRAG cuts indexing cost to about the same as vector RAG; DRIFT mixes global and local search. LightRAG v1.5.7 (2026-09). HippoRAG 2: no release since 2025-02.
- Criticism: "BM25 Wins at Scale" (arXiv 2607.26497, 2026-07-29): BM25 beats graph RAG beyond ~10M tokens; filesystem agents win on small corpora at 39× the tokens. GraphRAG-Bench (arXiv 2506.05690, ICLR 2026): graphs help multi-hop and summarization, hurt simple lookup (−13.4% on NQ).

## Agent memory
- **Zep/Graphiti** v0.31.2 (2026-10-20): episodes → LLM extraction → dedupe → edge invalidation. Backends Neo4j, FalkorDB, FalkorDBLite (embedded), Neptune; Kùzu deprecated.
- **Mem0 v3** removed graph memory from OSS (graph is platform-only); OSS is ADD-only. A retreat from extraction-heavy graphs. https://docs.mem0.ai/migration/oss-v2-to-v3
- **Cognee** v1.6.6 (2026-10-29): remember/recall/improve/forget, optional OWL ontology.
- **Letta**: filesystem + grep scored 74.0% on LoCoMo vs Mem0-graph 68.5%. https://www.letta.com/blog/benchmarking-ai-agent-memory
- Benchmarks are shaky: MemDelta (arXiv 2606.29914) found embedding swap alone moves accuracy 6.2 pts.

## Storage (checked 2026-11-04)
| Engine | State | Fit for one user |
|---|---|---|
| SQLite + recursive CTE + FTS5 + sqlite-vec | stable; mycelium ADR 0006 picks it | **best**: zero ops, diffable, already the brain |
| Kùzu | archived 2025-10-10; fork LadybugDB v0.22.1 alive | the warning: engines die |
| FalkorDB / FalkorDBLite | active; Lite spawns Redis, py3.12+ | ok if Cypher needed |
| Neo4j | mature, JVM server | too heavy |
| DuckPGQ | research WIP | wait |
| CozoDB | dormant | no |
| SurrealDB 4.0 | released 2026-11-03 | young |

## Code graphs for review
Greptile, CodeRabbit, Aider repo-map, Sourcegraph SCIP: all **deterministic** (parsers, not LLMs), which is why they work. **Preston** is mycelium's own PR reviewer over a belief graph. Hosted deploy destroyed 2026-09-25, being rebuilt local. Its own handoff says it "works as plumbing and fails as a reviewer".

## "Graph engineering"
The term exists but isn't settled. arXiv 2608.21156 (2026-08) uses it for explicit graphs of tasks/agents/state (orchestration). Vendors split it into an *execution graph* (what runs) and a *context graph* (what's known, decision traces). Recurring principles: schema-first core; deterministic ingest of structured APIs; LLM only for unstructured text; provenance on every edge; humans promote; evaluate edge precision, not just answers.

## Anti-patterns
Extraction hallucination (fix: anchor to source spans); duplicate-node bloat (weak resolution); over-ontologizing; stale edges that still retrieve; summaries replacing derivations ("premature compression"); paying extraction tokens for gains BM25 gives free; benchmark theatre.

## Mycelium model (local clone)
- Truth-maintenance system, not an association graph. One node type, `Claim`, content-addressed. Status: axiom (human), belief (derived), hypothesis (extracted).
- Justifications come in three kinds: provenance, inference, ratification. Retraction cascades; nothing is deleted.
- **Absence is data.** It records "looked, found nothing", "couldn't look" and "never looked" separately.
- The graph is a disposable pure function of its inputs. The LLM is quarantined at the edges. Each predicate family has a freshness contract.
- Copy (Codex agrees): claim vs evidence, ratification tier, retractions kept, explicit failed/unexamined coverage. Don't copy: the full truth-maintenance engine, dynamic ontology, platform sprawl.

## Principles for the fleet
1. SQLite stays; add `entity`, `alias`, `edge`/`assertion`, `evidence` tables + recursive CTEs.
2. Small fixed schema: Person, Project, Commitment, Decision, Thread, SourceDocument; edges `owes`, `owed_to`, `about`, `in_thread`, `evidenced_by`, `decided_in`, `depends_on`, `supersedes`. New types only by review.
3. Linear, GitHub, calendar and Notion edges come from code. Only mail, Slack and transcripts go through the LLM, and they land as `proposed`.
4. **No span, no edge.** Every assertion quotes its source. This is the anti-slop filter.
5. Bitemporal + supersede, never delete. Replies don't imply done; completion needs its own evidence.
6. Resolve entities by code first (exact ids/emails); fuzzy matches go to a human queue; merges must be reversible.
7. Retrieve with BM25/FTS first, then expand 1–2 hops. No communities or LLM summaries.
8. Score the graph itself: edge precision on a labelled sample, duplicate rate, stale-commitment count. Failed sync means freshness unknown, not empty.

## Unverified
Exact Mem0 graph-removal version; LoCoMo judge audit figures; most vendor memory scores; Cognee `temporal_cognify`; FalkorDBLite import path; any public Preston presence.
