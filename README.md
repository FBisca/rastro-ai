![Rastro: an open, pluggable personal assistant with evidence-backed memory](docs/assets/rastro-header.jpg)

# Rastro

*The trail behind everything your assistant knows.*

Rastro is an open, pluggable personal assistant. It keeps a cited understanding of a person's world (people, commitments, decisions, sources) and acts, proposes or asks on their behalf, learning over time which to do. Every claim leaves a *rastro*, a trail back to its evidence.

Status: concept. Nothing is built yet.

- North star: `docs/concept.md`
- Language: `docs/GLOSSARY.md`
- Decisions by scope: `docs/decisions/` (levels, learning, knowledge, vocabulary, sharing, plugins, surfaces, renderers, interruptions)
- Architecture decisions (ADRs): `docs/adr/`
- Roadmap: `docs/roadmap.md`
- Parked: `docs/open-questions.md`
- Research: `docs/research/`

## Let an agent search your evidence (MCP)

`rastro mcp` serves two read-only tools, `search` and `coverage`, to an agent tool over stdio. Run `rastro sync` first so there is evidence to search.

- Claude Code: `claude mcp add rastro -- rastro mcp`
- Codex: add this to `~/.codex/config.toml`:

  ```toml
  [mcp_servers.rastro]
  command = "rastro"
  args = ["mcp"]
  ```

Results are data from sources, never instructions, and each hit carries its trust class. These labels don't stop the agent tool from acting on what it reads: the agent's host still limits what the agent can do.
