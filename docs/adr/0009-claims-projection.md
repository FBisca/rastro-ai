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

## Amended 2026-11-05
Status stays accepted. Knowledge graph deep dive (`../research/kg-deep-dive/42-proposed-decisions.md`); product meaning in `../decisions/knowledge.md` §33–§48. This replaces the T3 projection tables, which were disposable and are dropped by a later migration (`0002`).
- **Tables (one migration, observed-only, about eight).**
  - `entity`: id and canonical identifier. No type column: a type is a `rastro:type` claim.
  - `claim`.
  - `justification` (renames *claim support*): id = hash(claim id, extractor id, derivation key), plus kind (provenance, inference or confirmation; only provenance is used now), extractor and version.
  - `justification_span`: the evidence spans of one derivation; all are required (AND). A justification is live only while every span is. The premise table arrives with the first inference.
  - `occurrence`: per justification, id = hash(justification id, opening event key); replaces *known interval*.
  - `claim_link`.
  - `extraction_checkpoint`.
  - `projection_publication`: insert-only log, outside the swapped tables.
  - Aliases are `rastro:alias` claims with an index on (`rastro:alias`, value); there is no alias table. Not in this migration: intents, rules, artifacts, merge, premise, valid-time and lens tables. Each arrives in its own additive migration with its own insert-only triggers and forget path.
- **Slot.** A scalar slot is (source, scope, location, field, subject, predicate). The field group for conflicts between extractors is the slot without the rule.
- **Portable evidence key** = hash(source, scope, location, field, start, end, quote, `captured_at`), computed at projection time and stored on the span. No hash uses a local evidence id.
- **Event order and occurrences.** Replay order is the event key (`captured_at`, portable evidence key); the local evidence id is only the local tie-break. Known intervals are half-open and may be zero-length; a closed occurrence and a recurrence at equal `captured_at` are separate rows. Claim presence is computed on event order.
  - *Corrected 2026-11-05, during the build:* ordering by the portable key can't be right, because a hash says nothing about which observation came first. At one `captured_at`, open → closed → open from one source would replay as open, open, closed (or closed, open, open). So **replay order is (`captured_at`, evidence append order)**: the local evidence id, across all pairs. That is portable because import keeps export order (`42` §D1 i), and it was this ADR's original replay order. The portable key is used only for identity (spans, occurrence ids, revision), never for order. **Occurrence id = hash(justification id, opening `captured_at` and portable key, recurrence ordinal)**. The ordinal counts the earlier occurrences of the same justification with the same opening key, in replay order, so two equal-key recurrences don't collide. No local id enters any hash. The lead decided this to unblock the build; the owner confirmed it on 2026-11-05.
  - *Build note:* at ordinal 0 the occurrence id hashes exactly like the two-field form (justification id, opening key). The ordinal enters the hash only from 1, so ids without a recurrence are unchanged. `claim.known_since` is the start of the claim's latest union known interval, not a single derivation's opening, so `about` can order by an index.
- **Revision v2.** Tag `rastro/revision/v2`: hash of status, the sorted, deduplicated set of visible support portable keys, link claim ids and validity. No versions, rule ids or local ids. This repairs a change made under the v1 tag. Entity and claim id encodings stay v1, and the revision still changes when a second support is added. The meta-claims are `rastro:type` and `rastro:alias`; the `rastro:` prefix is reserved to the core.
- **Valid time** is reported `unknown` by the API, with no column (replaces "stored as unknown").
- **Publication log.** Each publication inserts one `projection_publication` row: extractor and vocabulary versions and the evidence high-water mark. It is the only non-disposable table in the migration.
- **Publication P1.** The TEMP shadow-swap in one transaction, with explicit TEMP DDL instead of rewriting `REFERENCES`. Move to generation keys when a publication takes over 60 s or the WAL exceeds twice the database. The writer runs `ANALYZE` on the projection tables at publication (a read-only open cannot).
- **Scope mechanism.** A scoped read recomputes exactly from visible justifications and visible intents; stored full-scope state is only an acceleration for unscoped reads. Each occurrence carries (source, scope, location, field, `captured_at`, `source_updated_at`, portable key), indexed, plus an index to read a subject's or object's claims by recency. A benchmark gate (20k-claim hub, 100k-claim database, cold and warm, two scope sets, p95 under 200 ms) applies before the scoped read ships; if it fails, a cache for common scope sets is added, with the exact recompute as fallback and a work budget that fails honestly. Required tests: random scope *sets*, hidden-input invariance, per-scope rebuild equals scoped read.
- **Benchmark gate result, 2026-11-05.** On the fixture above, with the hub's type claim carrying one justification per item, cold scoped reads took 372–415 ms p95 for one scope and 633–668 ms for two. Warm reads took 1.5–3.6 ms. The fallback is in place: a process-local cache of 4 scope sets, keyed by database file identity, scope set, target and projection high-water marks, with exact recompute on a miss. A work budget counted in visible spans only fails with an error that carries no counts. The owner accepted the cold miss as a known limit (`../decisions/knowledge.md` §49). Candidate selection still runs over hidden claims, so timing can differ, within the accepted limit of `../decisions/surfaces.md` §21.
- **Retirement.** On an extractor version bump, old-version justifications are replaced in the same transaction as the final batch of the replay; a failed upgrade keeps the last good output (`0008`).
- **Considered and rejected:** 19–25 tables now; per-scope-set lenses; generation prefixes on every key (P2); hashing definition versions into the revision.

## Amended 2026-11-05 (claims hardening)
Status stays accepted. Product meaning in `../decisions/knowledge.md` §50–§52, `../decisions/vocabulary.md` §20–§22 and `../decisions/surfaces.md` §33 (the rebuild `pairs` result) and §35 (the displayed type, computed in the replay for scoped reads).
- **Rebuild carry-over (amends *Rebuild*).** A pair whose extractor never started (not installed, failed its check, scope forbidden) is carried into the new generation: its justifications, spans, claims, entities and checkpoint. Statuses, occurrences and links are recomputed in full for every touched component, so a claim justified by a carried and a rebuilt pair equals the incremental projection. An extractor that starts and fails still aborts the rebuild and keeps the old projection. The result reports each pair.
- **Carried output is re-derived.** It is not precious; it is replaced when the extractor returns and its pair replays. "Rebuild equals incremental" is stated for when every pair replays.
- **Entity-wide type conflict.** `conflicts_with` between `rastro:type` claims live anywhere in a component is computed in the lifecycle rollup over the component already loaded, unless one type is an ancestor of the other. Dominance, `supersedes` links and incremental slot selection stay per (location, field), so the hub scaling bound holds. The links change, so a `claims rebuild` or the next sync recomputes them; no schema migration.
- **Scope mechanism for aliases.** A scoped alias lookup starts from the matching `rastro:alias` claims with at least one fully visible justification. No stored status takes part in root selection: hidden roots would feed the work budget, and a hidden supersession would drop a visible root. The status set, ambiguity and `current`/`historical` marks are applied inside the replay from visible justifications.
