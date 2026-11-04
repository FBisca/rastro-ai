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
8. **Implementation language: Rust for the core** (decided by the owner 2026-11-04). Plugins run as separate processes over a versioned, language-neutral protocol, so contributors can write them in any language. Agent tools are driven as child processes, the way `hc` drives them. This follows Codex's recommendation, which noted the main risk: a reliable plugin host costs more to build than calling functions in one process.
