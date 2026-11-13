---
status: accepted
date: 2026-11-04
---

# Plugin manifest: `rastro-plugin.toml`; the manifest declares, grants narrow

Each plugin ships a `rastro-plugin.toml`. It declares:
- `id`, `name`, `version`, `author`
- `kinds`
- `protocol` (the major version supported)
- `command`
- `permissions` (`submit`, `scopes`, `actions`)
- `triggers`
- `dependencies`

Fields a plugin doesn't use are written explicitly empty. TOML because owners read and edit it by hand, as with herdr plugins.

The manifest is a request, not an authorization. Registering a source creates a **grant** that can only narrow the manifest, and it binds the source's scope. The core rejects any submission that is outside the grant: a kind it doesn't permit, a different scope, a different source.

## Consequences
- Approving permissions at install, and showing what changed in them on update, comes with roadmap slice 15. Until then, registering a source is the approval.
- A connector that reads untrusted content declares `actions = []`, and the core refuses to grant it any (`decisions/plugins.md` §6).
- `permissions.env` lists the environment variables a plugin needs (for example `GH_TOKEN`). The core starts every plugin with a cleared environment: `PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, plus those listed. A source whose scope isn't in `permissions.scopes` is refused before spawning (added 2026-11-04).

Amended 2026-11-04 by ADR 0008: a manifest may also carry `extraction_rules`, `vocabulary` and, on a connector, `default_extractor`, and `kinds` may include `extractor` and `vocabulary`. They are optional; absent means empty, so existing manifests stay valid unchanged. Status stays accepted.

Amended 2026-11-13 by ADR 0012: `triggers` and `dependencies` may be absent (empty), and a plugin of kind `agent-tool-adapter` carries `agent_tool`, the capabilities it needs from an agent tool, never a named tool. Status stays accepted.
