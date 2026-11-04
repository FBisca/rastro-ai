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
18. **"Couldn't look" reasons** are typed: `auth`, `forbidden`, `not_found_or_hidden`, `rate_limited`, `network`, `timeout`, `protocol`, `interrupted`, `storage`, `result_cap` (the source caps how many results a query returns, so the read is incomplete; 2026-11-04), `not_installed` (the source's connector isn't installed; 2026-11-04), each with optional safe detail and a retry time. An incomplete read never becomes "empty".
19. **Re-observing unchanged content adds no evidence;** changed content appends. Encoding: `../adr/0005-evidence-observations-and-spans.md`.
20. **An empty search is not an empty source.** Search results always state the coverage of the sources they searched.

## Added 2026-11-04 (roadmap slice 2)
21. **Coverage is per (source, scope) pair, across config and storage.** The pairs are those in the config plus those only in storage (a removed source, or one whose scope changed). A pair that isn't in the config is `registered: false`. Its coverage comes from its stored windows: once the freshness rule has passed it reads "unknown", and a pair that never synced stays "never looked". So a hit always states its coverage (§20), even for a removed source. This decides the hit half of open question #9; the `source remove` command is still open.

## Accepted limits 2026-11-04
22. **Sources are keyed by name.** An old (source, scope) pair takes the source's current freshness rule, rather than a historical rule for that pair. While `rastro mcp` runs, its read-only reader uses fresh config without projection; the CLI projects config into storage. After a config edit, their answers for an old pair can differ until a writing open projects the change.

## Added 2026-11-04 (roadmap slice 3)
Architecture: `../adr/0008-extractor-role-and-protocol.md`, `../adr/0009-claims-projection.md`.
23. **Claims are observed only, from structured evidence.** An extractor's deterministic rules, checked by the core, are the only source. Hypotheses wait for slice 5. No claim comes from a title or body.
24. **One exact canonical identifier is one entity,** across sources and scopes. That is not a merge (§4; merges are slice 4). No fuzzy or name matching, no bare-login lookup, no `about(me)`.
25. **Known time comes from evidence; valid time is `unknown`.** A known interval opens at the `captured_at` of the observation that establishes support and closes at the `captured_at` of the one that replaces it. An item-level update time is not when a field changed, so valid time isn't fabricated (§7). "As of" queries are slice 7.
26. **Supersession and conflict.** A claim is `observed` while any current support remains, and `superseded` once none does. `superseded_by` is set only for a scalar replacement. A missing or null field, an unreadable item or an item not returned never counts as removal. When sources disagree about the same resource and field, the newer comparable observation is current; a tie, or observations that can't be compared, leaves both `observed` with `conflicts_with` (§8). Conflict and supersession are computed from visible supports only (§14).
27. **Bots are kept.** Bot comments stay evidence and bots get author claims; filtering is for slice 5. Authors are recorded as structured evidence. Decides open questions #6 (for this slice) and #7.
28. **`rastro sync <source> --reread`** re-reads from the source's registered window start, under the normal append, unchanged and coverage rules. It can't recover past edits or items that left the selection, and an item not returned is never taken as deleted. CLI-only.
29. **Answers state extraction state** next to coverage: `not_installed`, `behind`, `running`, `failed` or `current`. An empty `about` is not "nothing known" (§11, §20).

## Accepted limits 2026-11-04 (roadmap slice 3)
30. **Repository rename or transfer, and URL redirects,** give a new canonical identifier, so a new entity.
31. **List removal is unsupported.** List evidence stays per-index (`../specs/evidence.md` R38), so a removed element is not seen as removed.
32. **Rank leakage across scopes remains** as in §21 of slice 2; a scope isn't an isolation boundary until slice 14.
