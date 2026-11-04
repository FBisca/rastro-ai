---
status: accepted
date: 2026-11-04
---

# Claims are a materialized projection of evidence, swapped by generation

Claims, entities and aliases are derived from evidence by the extractors of `0008-extractor-role-and-protocol.md`, and stored as a **projection** in the same database (`0002-sqlite-fts5-storage.md`). Like the FTS table it is disposable: `rastro claims rebuild` restores it from evidence, and it is never the only copy of anything (`../decisions/knowledge.md` §12). It is written on the write path; the read path only reads, as in the FTS precedent.

## Tables
- **entity:** id, type, canonical identifier.
- **alias:** entity id, alias, kind (a login, a URL).
- **claim:** id, subject, predicate, object (entity or typed literal), status, revision.
- **claim link:** from claim, to claim, kind (`conflicts_with`, `supersedes`). `superseded_by` is the inverse of `supersedes`; it is set only for a scalar replacement.
- **claim support:** claim, evidence id, span, extraction rule; a `current` flag per (source, scope, location, field, extraction rule). The claim rolls these up: `observed` while any support is current, `superseded` once none is. History is kept.
- **known interval:** claim, start, end (open while current). Start is the `captured_at` of the evidence that establishes support; end is the `captured_at` of the evidence that replaces it. **Valid time** is stored as `unknown`: a source's item-level update time isn't when a field changed. `source_updated_at` stays support metadata.
- **extraction checkpoint:** per extractor version and (source, scope), the last evidence processed, in replay order.

These tables live in a **generation**; readers see the active one.

**Extractor runs** are not part of the projection. They are rows in the existing `runs` table (plugin id, version, counts), outside any generation, so a rebuild never discards the log (`../decisions/plugins.md` §9). They stay separate from connector coverage.

## Ids and revision
- A **versioned canonical encoding** feeds a hash. Entity id = hash(canonical identifier). Claim id = hash(subject id, predicate, typed object, with entity and literal tags). Rebuilding gives the same ids.
- **Revision** = hash of the claim's visible state: status, the set of current supports, conflict and supersession links. A second support changes it. The stored revision is the full-scope value; a scoped read recomputes it from the supports visible to that scope. A→B→A keeps the claim id and records a new interval and revision. Renderers target a claim id plus the revision seen (`../decisions/renderers.md` §5).
- **Replay order** is (`captured_at`, evidence id).
- **Scope:** every derived field (aliases, types, status, revision, links, counts, extraction state) is computed from supports visible to the caller, so a launch `--scope` filters metadata, not only rows.

## Writing
- Every writing `sync` processes all evidence pending past the checkpoints, including evidence a failed sync kept and work a crashed extractor left, even when the sync accepted nothing new.
- Each batch and its checkpoint commit in one transaction.
- A missing or null field, an unreadable item or an incomplete record never closes a support.
- **Rebuild** replays all evidence into a new generation and swaps it in atomically. A failed rebuild keeps the old projection; a reader sees the old or the new generation, never a mix. Extraction failing after ingestion leaves the last good projection readable.

## Considered options
- **Deriving claims on each read:** the read path can't write, and cost grows with evidence.
- **Extracting only a successful sync's new evidence:** a crash or failed sync would lose claims.
- **Row ids for claims:** rebuild would renumber them.
- **A count of status changes as the revision:** two different states can share a count.
- **`valid_from = source_updated_at`:** fabricates valid time.

## Consequences
- A sync holds the write lock a little longer; WAL readers aren't blocked.
- Rebuild is O(evidence); fine at the current scale.
- A new extractor or vocabulary version means replay (`../decisions/vocabulary.md` §6).
