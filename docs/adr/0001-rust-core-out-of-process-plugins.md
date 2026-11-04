---
status: accepted
date: 2026-11-04
---

# Rust core, plugins as separate processes

The core is a Rust library, and every surface (CLI, MCP server) is a thin layer over it. Plugins run as separate processes and talk to the core over a versioned, language-neutral protocol, so contributors can write them in any language. Agent tools are driven as child processes, the way herdr's `hc` drives them. The owner chose this on 2026-11-04, following Codex's recommendation.

## Considered options
- **Plugins loaded in-process (dynamic libraries, WASM):** cheaper to call, but they tie plugins to one language or runtime, and a crashing plugin takes the core down with it.
- **A core in a scripting language (the prototype was Python):** faster to start, but weaker as a library others embed, and harder to ship as one binary on macOS and Linux.

## Consequences
- A reliable plugin host (spawning, handshakes, timeouts, crash isolation) costs more to build than calling functions in one process. This is the main risk.
- Plugin permissions are enforced at the process boundary: the core validates every message a plugin sends.
