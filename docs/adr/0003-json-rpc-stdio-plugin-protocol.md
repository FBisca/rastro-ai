---
status: accepted
date: 2026-11-04
---

# Plugin protocol: JSON-RPC 2.0 over stdio

The core spawns each plugin as a child process and talks JSON-RPC 2.0 over its stdin and stdout, one JSON object per line. The plugin writes its logs to stderr. Every language can do this, it's the shape MCP uses, and it matches ADR 0001.

## Shape (protocol 1.x)
1. **Core → plugin `initialize`:** protocol version, manifest id, and the run binding (run, source, scope, selector, window). The plugin answers with the version and manifest id it speaks. If the major versions differ or the manifest id doesn't match, the run fails before any evidence is ingested.
2. **Plugin → core:** `evidence.submit` and `coverage.report` as notifications. Source, scope and window are inherited from the binding. A message that tries to set them differently is rejected.
3. **Plugin → core `run.finish` (request):** the plugin's status and how many items it submitted. The core answers with its own counts of accepted, rejected and unchanged items. The core's counts are the authority.

## Failure rules
- If the process exits without a `run.finish`, the run fails as `interrupted`.
- A message that is malformed or over the size limit, or a plugin that goes silent past the timeout, gets the process killed. The run fails as `protocol` or `timeout`.
- Evidence already accepted stays. Coverage for the window becomes "couldn't look", never "empty".
- An unfinished run left by a crashed core is marked `interrupted` the next time the core starts.

## Considered options
- **gRPC or another binary RPC:** typed, but heavier for contributors writing an afternoon plugin.
- **Bare NDJSON with no RPC envelope:** simpler, but it has no request/response pairing for the handshake and the finish.
