---
status: accepted
date: 2026-11-04
---

# SQLite with FTS5 behind a storage trait

The core stores evidence, coverage, runs and sources in one embedded SQLite database (`rusqlite`, bundled), with an FTS5 table for text search. All access goes through a domain storage trait in `rastro-core`, so the engine can be replaced. For one owner it means nothing to run or maintain, it ships in the binary, and it covers text search and 1–2 hop link walks (recursive CTEs). See `../research/knowledge-graphs-2026-11.md`.

## Considered options
- **A graph database (embedded or server):** strong for multi-hop queries we don't need yet. The research also records an embedded graph engine dying in 2026.
- **Append-only files (JSONL) plus a separate search index:** simple, but they'd need their own transactions and query layer.

## Consequences
- **Invariants:**
  - evidence rows are only inserted, never updated (forgetting is the one exception, and it leaves a marker);
  - writes from one plugin message are a single transaction;
  - the FTS5 table is a disposable projection, rebuilt from evidence by `rastro index rebuild`, and never the only copy of anything.
- The database lives under the data directory, never in a checkout.
