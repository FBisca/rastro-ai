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
- Migrations are named `YYYYMMDDHHmm_<name>.sql` (UTC timestamp, short snake_case name) and recorded by name in a `schema_migrations` table, so parallel branches never collide on a number. Pending migrations apply in timestamp order; a database holding a migration this binary doesn't know is refused. Applied migrations are never edited (added 2026-11-04).

## Amended 2026-11-04
Status stays accepted. Roadmap slice 2 adds a reader that runs beside a sync (`0006-mcp-server-over-stdio.md`).
- **WAL mode.** SQLite runs in WAL mode, switched on by the writing open. A sync no longer blocks reads. An existing database switches on its next writing open; that's a mode change, not a migration, and one pragma turns it off.
- **A read-only open path** next to the writing one. MCP tool calls use it; the CLI keeps the writing open:
  - no migrations, no source projection, no recovery;
  - config is read fresh on each open, under a shared config lock (the writing open keeps the exclusive one);
  - one read transaction per call, so the hits and the coverage in one answer come from the same snapshot;
  - a database holding a migration this binary doesn't know, or a pending one, is refused.
- **Unfinished runs.** A read that finds an unfinished run while no sync holds `sync.lock` answers "unknown: interrupted" for that source and writes nothing; startup recovery stays on the writing open. A live sync's unfinished run doesn't change the answer.
- **A missing database** answers from config (every source "never looked") and no file is created.
- **Locking.** A reader takes `sync.lock` shared and holds it only until its snapshot is pinned. A missing `sync.lock` is probed again after the pin. The writing open retries `try_lock` briefly while only readers hold the lock, and skips recovery while a sync is live.
- **Files.** The read-only open never creates the database file. SQLite may create `-wal` and `-shm` beside it; where it can't, the read is refused as not readable.
