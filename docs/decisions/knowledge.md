# Knowledge of evidence

Status: agreed 2026-11-04. North star: `../concept.md` §1. Research: `../research/knowledge-graphs-2026-11.md`.

## Decisions
1. **Three layers.** Sources → **Evidence** (never changes) → **Knowledge** (claims about entities, derived and rebuildable).
2. **Evidence never changes.**
   - Each piece records its source, location (ID, URL or offset), capture time, the quoted span, and its trust class.
   - Forgetting (privacy) is the only removal, and it leaves a marker behind.
3. **Keeping raw content is the owner's choice per source:** the full original, or a pointer plus the span.
4. **One identity per entity, with many aliases.**
   - A merge is itself a claim, so it can be undone.
   - Matches on exact identifiers merge automatically. Fuzzy matches become an Ask. Names alone never merge.
5. **Claims.**
   - A claim is a statement about entities, with a status (`observed`, `hypothesis`, `confirmed`, `retracted`, `superseded`) and at least one justification (provenance, inference or confirmation).
   - When two sources say the same thing, that's one claim with two supports, not a duplicate.
6. **No span, no claim.** A claim drawn from unstructured content needs a quoted span from its evidence.
7. **Two times.** Valid time (when it's true in the world) and known time (when Rastro learned or unlearned it). "As of" queries are supported.
8. **Contradiction.** Conflicting claims both stay, and the conflict is visible. It gets resolved by newer evidence, by source authority, or by an Ask.
9. **Source authority.** Plugins declare defaults per kind of claim (for example "the tracker beats chat on task status"). The owner can override them.
10. **A retraction carries through** to:
    - derived claims;
    - pending proposals;
    - Action runs. An Action run whose basis was retracted is flagged, and an undo is proposed.
11. **Absence is knowledge.** Coverage records "found", "empty", "couldn't look" (with the reason) and "never looked" per source, scope and window. Past a source's freshness rule, the answer is "unknown", not "nothing".
12. **Knowledge can be rebuilt.** Knowledge = f(evidence, confirmations, rules). Swapping an extractor means replaying the evidence, not migrating data. Evidence, confirmations, rules and outcomes are the only precious inputs.
13. **Completion needs its own evidence.** A reply, or a related item reaching Done, doesn't prove a commitment was met.
14. **Scopes.** Evidence and claims carry scopes. Plugins and renderers see only the scopes they're granted. Forgetting carries through to everything derived from it.
15. **Retrieval.** Text search first, then expand 1–2 links through the graph. Summaries never replace the evidence.

## Added 2026-11-04 (roadmap slice 1)
16. **Raw content default:** pointer plus quoted span. Keeping the full original is opt-in per source, at registration. Replay covers only what was retained.
17. **A coverage window** is a half-open interval over the source's update time, plus a selection. A traversal cursor is how a connector resumes, not what coverage means. A window is complete only once every page and dependent read has finished.
18. **"Couldn't look" reasons** are typed: `auth`, `forbidden`, `not_found_or_hidden`, `rate_limited`, `network`, `timeout`, `protocol`, `interrupted`, `storage`, each with optional safe detail and a retry time. An incomplete read never becomes "empty".
19. **Re-observing unchanged content adds no evidence;** changed content appends. Encoding: `../adr/0005-evidence-observations-and-spans.md`.
20. **An empty search is not an empty source.** Search results always state the coverage of the sources they searched.
