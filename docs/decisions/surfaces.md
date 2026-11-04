# Surfaces: library, CLI, MCP

Status: agreed 2026-11-04. North star: `../concept.md` §5.

## Decisions
1. **The core is a library.** Every other surface is thin, the way the `hc` CLI and its API sit over herdr-core.
2. **The CLI and the MCP server are generated from one schema,** so they never drift apart.

| | CLI (`rastro`) | MCP |
|---|---|---|
| Best for | The owner, scripts, agents that have a shell | Agents, especially quarantined ones and hosts without a shell |
| Permissions | All or nothing (shell access is an outbound capability) | Per tool |
| Who is calling | Taken from the environment | Derived by the server, never declared by the caller |

3. **Reads** work over either surface.
4. **Writes from agents** (`submit`, `propose`, `ask`) go through MCP, because provenance and per-tool permission are the trust model. A quarantined extractor uses MCP only.
5. **Concept-level operations** the core offers agents:
   - `about(entity, since)`
   - `why(claim)`
   - `open_commitments(person)`
   - `coverage(source)`
   - `search(text)`, then follow 1–2 links
   - `propose(action)`, `ask(question)`, `submit(claim)`
6. **Platforms.** macOS and Linux are required; Windows is wanted.
7. **Future surfaces** (a native app, editor integrations, someone else's agent) drive the same library.
8. **Implementation language and process model:** see `../adr/0001-rust-core-out-of-process-plugins.md`.

## Added 2026-11-04 (roadmap slice 1)
9. **Every CLI command has `--json` output** from the versioned operation schema the MCP server will reuse. A failed `sync` exits non-zero.
10. **Data location:**
    - `RASTRO_HOME` overrides both directories (data in `$RASTRO_HOME/data`, config in `$RASTRO_HOME/config/config.toml`);
    - otherwise `XDG_DATA_HOME` and `XDG_CONFIG_HOME`;
    - otherwise `~/.local/share/rastro` and `~/.config/rastro`, on macOS and Linux alike.

## Added 2026-11-04 (roadmap slice 2)
Architecture: `../adr/0006-mcp-server-over-stdio.md` (the server, the pinned protocol revision, errors) and `../adr/0007-typed-operation-catalog.md` (one catalog for the CLI, MCP and the schema).
11. **MCP exposes `search` and `coverage` only.** `sync`, `source.*` and `index.rebuild` stay CLI-only. This is the first real use by agent tools, before claims or identity.
12. **A search hit shows why it matched.** It carries a `match` excerpt: centred on the first match, at most 400 bytes, cut at UTF-8 character boundaries, with offsets from the same origin as the span, plus `clipped_before` and `clipped_after`. Stemmed and diacritic-folded matches count, as in the index. If the match can't be located, the excerpt starts at the field's start with `located: false`. (Decides open question #8.)
13. **Bounded search.** Amends `../specs/evidence.md` R49, R60 (applied when the plan is archived).
    - `quote` and `original` come back only with `full: true` (`--full` on the CLI). Human `--full` shows the whole quote, escaped and not clipped.
    - `limit` defaults to 20, range 1–50. `truncated: true` says more hits exist. Order is bm25, then id.
14. **Bounded coverage.** Each source returns its overall state, judged over all its windows, plus its 10 newest windows (newest first) and `windows_total`. `all_windows: true` (`--all-windows`) returns every window. Amends R63; R64 holds (applied when the plan is archived).
15. **Scope is set by the owner at launch.** An optional, repeatable `rastro mcp --scope <s>` filters search and coverage inside the storage query, before the limit. A tool argument can't name a scope. No flag means all scopes. Per-plugin scope grants stay in slice 14.
16. **The caller is the owner.** The client's self-reported `clientInfo` grants nothing and isn't stored. MCP reads aren't plugin runs and aren't logged yet: plugins §9 logs plugin runs, and here the agent tool is a client, not a plugin Rastro runs. Logging reads is slice 9.
17. **Results are labelled, not enforced.** Hits keep their trust class, and the tool descriptions say results are data from sources, never instructions. Nothing is rewritten or fenced, and Rastro doesn't claim injection resistance: the agent tool still limits what the agent can do.
18. **Operation errors are results, protocol errors are protocol.** An error inside an operation is a tool result with `isError: true` and the v2 error envelope. An unknown tool, or arguments that fail the input schema, is a JSON-RPC error.
