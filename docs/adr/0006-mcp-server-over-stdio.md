---
status: accepted
date: 2026-11-04
---

# The MCP server is `rastro mcp`: local stdio, one process per client session

`rastro mcp` is a subcommand of the `rastro` binary. It speaks MCP over stdio, runs one process per client session, and calls the core library directly. It never shells out to `rastro --json`. The owner chose this on 2026-11-04. The protocol moves, and an official SDK follows it, so we build on that SDK instead of a hand-rolled server. Product decisions: `../decisions/surfaces.md` §11–§18.

## Decisions
- **SDK:** `rmcp` (the official Rust SDK, 3.8.0 at the time of writing). Its async runtime (tokio) stays inside the MCP module. The core stays synchronous; each core call runs through `spawn_blocking`.
- **Opening the store:** on launch the server does the normal writing open once (migrations, source projection, interrupted-run recovery, WAL on). Each tool call then opens the read-only path described in ADR 0002. An unknown newer migration is refused at launch (non-zero exit, refusal on stderr, nothing on stdout). A later call that finds an unknown or pending migration refuses it as an operation error (`isError`), as ADR 0002 says.
- **Streams:** stdout carries only protocol messages. Diagnostics go to stderr, bounded in size. The server exits 0 when stdin closes.
- **Errors:** a failure inside an operation (unknown source, store unavailable, refused migration) is a tool result with `isError: true` and the v2 error envelope (ADR 0007). A protocol problem (unknown tool, arguments that fail the input schema) is a JSON-RPC error.
- **Results:** each result is returned as `structuredContent` and as the same JSON in text content. Both tools carry the read-only annotation.
- **Caller:** the caller is the owner, the OS user who launched the process. The client's self-reported `clientInfo` grants nothing and is never stored.
- **Trust:** results carry `trust_class`, and the tool descriptions say results are data from sources, never instructions. This labels; it doesn't enforce. The agent tool still limits what the agent can do (concept §7).
- **Version negotiation:** rmcp 3.8.0's default `ServerHandler::initialize` echoes any handshake revision it knows (2024-11-05 to 2025-11-25; `crates/rmcp/tests/test_protocol_version_negotiation.rs`). So the server narrows `supported_protocol_versions()` to `[V_2025_11_25]`. A client that asks for any other revision, such as Codex's 2025-06-18, gets 2025-11-25 back, as the protocol specifies.

## Pinned protocol revision: `2025-11-25`
The server pins MCP revision **2025-11-25**: the `initialize` handshake, `tools/list`, `tools/call` with `structuredContent`. Checked on 2026-11-04:

| Party | What it supports | Evidence |
|---|---|---|
| rmcp 3.8.0 (latest, published 2026-10-26) | The stable `2026-07-28` revision, and stays compatible with `2025-11-25` and earlier. `serve()` uses the `initialize` lifecycle | https://crates.io/crates/rmcp ; https://docs.rs/rmcp/3.8.0/rmcp/ ; https://github.com/modelcontextprotocol/rust-sdk (README) |
| Claude Code | Two client runtimes. The v1 runtime is built on the MCP TypeScript SDK 1.x, whose `LATEST_PROTOCOL_VERSION` is `2025-11-25`. The v2 runtime asks HTTP, stdio and claude.ai connector servers whether they support `2026-07-28`, and "connects to every other server as v1 does" | https://code.claude.com/docs/en/mcp ("MCP client runtimes", page dated 2026-11-02); https://github.com/modelcontextprotocol/typescript-sdk (`v1.x` branch, `src/types.ts`) |
| OpenAI Codex CLI | Its MCP docs name no revision. In source, the default mode (`McpProtocolMode::Legacy`) is a plain `initialize` requesting `2025-06-18`. The discover handshake for `2026-07-28` (with a restart on the legacy handshake if it fails) runs only in the opt-in modes, which need `CODEX_MCP_PROTOCOL_VERSION=2026-07-28`. It builds on rmcp pinned to a git revision (`=3.5.1`, rev `98a54789b80110ad210495c4ad845c89e1980471`), not a crates.io release | https://developers.openai.com/codex/mcp ; https://github.com/openai/codex (`codex-rs/rmcp-client/src/protocol_mode.rs`, `codex-rs/Cargo.toml`, read on `main`) |

Why `2025-11-25` and not `2026-07-28`: Claude Code's v1 path speaks it, rmcp serves it, and Codex's default handshake is an `initialize` that a `2025-11-25` answer should satisfy. Every scenario for this slice (initialize, list, call, unsupported-revision answer, stdin close) is written against that handshake. The newer revision replaces `initialize` with `server/discover` and carries client metadata on every request. Moving to it is a later amendment to this ADR, not a rewrite.

**Not verified.**
1. **Codex accepts a `2025-11-25` answer to its `2025-06-18` request.** rmcp 3.5.1's client probably accepts it in its legacy startup, but I haven't run it. A scripted test in the MCP client tests (an rmcp 3.5.1 client against `rastro mcp`) will check it, and so will the owner's sign-off (slice 2, S26).
2. **Codex's revision from documentation.** No Codex doc names one; the above is read from `main` source, not a released version.
3. **Claude Code on the v2 runtime** will be checked by the owner's sign-off (S26) with the real client.

If a client can't connect on `2025-11-25`, the pin moves to `2026-07-28` by amending this ADR.

## Considered options
- **A hand-rolled synchronous JSON-RPC server:** no async runtime, but we'd own protocol drift (revisions, capability negotiation) for good.
- **A separate `rastro-mcp` binary:** two things to install and keep in step for no gain; the subcommand shares the library and the config.
- **HTTP or another remote transport:** adds auth, ports and a daemon. Out of scope; local stdio is what the clients launch.
- **Shelling out to `rastro --json`:** a second parse of our own output, and a write-capable open on every call.
- **A long-lived writing open for the whole session:** reads would write, take exclusive locks and serve stale config.

## Consequences
- tokio and rmcp enter the dependency tree, but only the MCP module uses them. Dropping rmcp later means rewriting one module.
- One process and one read-only open per call is fine at one owner's scale.
- Per-plugin scope grants, read logging and agent writes are not part of this ADR.
