# Knowledge graph deep dive (2026-11-04)

Working folder for the graph-engineer crew. The owner (fbisca) makes every decision; agents research, design options and critique. Nothing here is decided until it lands in `docs/decisions/` or `docs/adr/` with the owner's agreement.

## Why
Slice 3 (`docs/plans/2026-11-04-claims-and-about.plan.md`) is paused, see its "Pause" section. The knowledge database and graph are the core of the library, and the owner wants to understand and choose each implementation detail.

## Read first (every agent)
- `AGENTS.md`: repo rules. Use the terms in `docs/GLOSSARY.md` exactly.
- `docs/concept.md`: the north star. Any option that contradicts it has to say so explicitly.
- `docs/decisions/knowledge.md`, `docs/decisions/vocabulary.md`, `docs/decisions/plugins.md`
- `docs/adr/0002-sqlite-fts5-storage.md`, `docs/adr/0005-evidence-observations-and-spans.md`, `docs/adr/0008-extractor-role-and-protocol.md`, `docs/adr/0009-claims-projection.md`
- `docs/research/knowledge-graphs-2026-11.md`: earlier research. Extend it; don't repeat it.
- The paused plan above: decisions D1–D24 and the Pause section.

## Decision areas (open)
- A1 **Data model:** RDF (triples, quads, RDF 1.2 triple terms) vs LPG vs relational claim tables vs hypergraph or n-ary relations. How a claim with several supports, a status, intervals and links is represented.
- A2 **Claim reification and provenance:** claim → support → evidence span; justification kinds (provenance, inference, confirmation).
- A3 **Temporal model:** valid time and known time, intervals, "as of" queries (slice 7), supersession.
- A4 **Identity and merges:** canonical ids, aliases, merges as claims that can be undone (slice 4), Persons vs accounts.
- A5 **Vocabulary and schema:** meta-model, namespaced vocabularies, domain and range, cardinality, validation (SHACL-like?), versioning and replay.
- A6 **Traversal and indexes:** 1–2-hop retrieval (knowledge §15), path queries, graph indexes.
- A7 **Store and engine:** stay on SQLite, add an embedded graph or RDF engine, or a hybrid. It has to be Rust-embeddable and local-first, and the projection has to stay rebuildable.
- A8 **Several extractors on one entity:** combining claims, authority, conflicts.
- A9 **Hypothesis and confirmed claims:** the owner's confirmations as precious inputs; retraction carrying through.
- A10 **Retrieval for agents:** how agents query the graph over MCP (`about`, `why`, search plus expansion).
- A11 **Fate of T4/T5:** keep, rework or drop the branches `feat/claim-lifecycle` and `feat/github-extractor`.

## Steps
0. Ground truth: `00-current-state.md`
1. Research: `10-models.md`, `11-time-provenance.md`, `12-engines.md`, `13-identity.md`, `14-retrieval.md`, `15-llm-ontology.md` (added at the owner's request)
2. Options: `20-arch-semantic.md`, `21-arch-pragmatic.md`
3. Cross-review: `30-review-of-semantic.md`, `31-review-of-pragmatic.md`
4. Brief: `40-decision-brief.md`

## Rules for agents
- Don't decide. Present options, trade-offs and evidence. Mark your own leaning as "leaning" if you have one.
- Cite sources: a URL plus the date you checked it, or `path:line` for repo facts. Put anything you couldn't verify under "Unverified".
- Write only your own file in this folder. Don't edit code, decisions, ADRs or other agents' files. Don't commit, push or merge.
- Be concise: tables and bullets over prose. The owner reads all of it.
