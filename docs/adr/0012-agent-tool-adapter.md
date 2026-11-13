---
status: accepted
date: 2026-11-13
---

# Agent tool adapter: a plugin kind that lets an agent runtime do judgment

The commitment path needs a model's judgment over untrusted text. ADR 0011's configured one-shot command did that without being a plugin. This ADR makes the judgment path the plugin kind `decisions/plugins.md` §2 already names, **agent tool adapter**, so no agent runtime is special to the core (§10). Product meaning: `../decisions/plugins.md` §29–§33.

## Shape
1. **Kind and manifest.** A plugin of kind `agent-tool-adapter` declares, in `rastro-plugin.toml`, `agent_tool = { capabilities = [...] }`: what it needs from an agent tool, never a named tool (`decisions/plugins.md` §4). The one capability today is `judge_text`. The core refuses the manifest before it starts anything when: the kind is not declared; `agent_tool` is missing or declares no capability; a capability is not one the core knows (so a tool's name there is refused); `agent_tool` holds any other field (`tool = "..."`); the protocol major differs; or the plugin holds an action permission (`decisions/plugins.md` §6: a plugin that reads untrusted text holds no actions). `triggers` and `dependencies` are optional in a manifest and read as empty when absent, as `extraction_rules` already are (ADR 0008).
2. **Binding.** `config.toml` holds `[commitment_adapter]` with `plugin` (the adapter's id) and optional `timeout_seconds` (default 120). It replaces `[meeting_extractor]`. Configuring it is the owner's authorization to send transcripts to that plugin; it is offered to nothing but `meeting.import`.
3. **Protocol (ADR 0003, minor-additive).**
   - `initialize`: params `{protocol, manifest_id, role: "agent_tool_adapter"}`, answered with `{protocol, manifest_id}`. An adapter holds no run binding.
   - `judgment.request` (core to adapter, one request, `id` 2): params `{instructions, owner_speaker, turns: [{turn, speaker, text}], fields}`. `instructions` are the core's own; the transcript appears only in `turns[].text`; `fields` names what every candidate must have.
   - Its answer is a JSON-RPC response whose `result` is `{candidates: [{turn, kind, responsible, recipient, promise, stated_date, quote}]}`. `kind` is `ask`, `promise` or `waiting_on`; `responsible`, `recipient` and `stated_date` may be null. Unknown fields are refused, as is a missing `turn`, `kind`, `promise` or `quote`.
   - A JSON-RPC `error` answer fails the run. No `run.finish`: the core closes the adapter's stdin when it has the answer and stops its process group.
4. **Strict validation.** A line that is not a JSON-RPC message, an answer that is not the shape, any message but the one response, or a candidate with an unknown `kind` fails the run as `extractor_output_invalid`; the last good results stay, marked stale (ADR 0011 §5). An adapter that is not installed, is refused, can't start, exits early, sends an error or exceeds its timeout fails it as `extractor_failed`. Past the shape, the checks of ADR 0011 §4 are unchanged and made by the core: the quote is an exact contiguous piece of the named turn (otherwise `invalid_quote`), the owner's involvement is supported by the transcript (otherwise the candidate is excluded and counted), and a stated date is a piece of the quote (otherwise it reads unknown). The adapter's `kind` is kept with the recorded output (ADR 0011's amendment); output recorded before it existed replays without one.
5. **Process.** The core runs the adapter as it runs any plugin: its own process group, a cleared environment plus `PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, what its manifest lists in `permissions.env`, and `RASTRO_HOME` when the core has it. Lines over 4 MiB, silence past the timeout, or a closed output fail the run. Its stderr is relayed like a connector's.
6. **Reference adapter.** `rastro-adapter-claude-code` lets Claude Code do the judgment: it runs `claude -p` with a JSON schema for the answer and every tool, setting, MCP server and slash command turned off, and relays the object Claude Code returns. It is installed by hand, as the connectors are (plugin install is slice 15).

## Considered options
- **Keep the one-shot command, add `kind` to its response.** Rejected: it stays outside the plugin model (no manifest, no quarantine check, no kind), so a second agent runtime would be a second special case.
- **Let the manifest name the tool it drives.** Rejected: §4 says capabilities, never a named tool; the owner picks the runtime by choosing which adapter to install.
- **A long-lived adapter process serving many requests.** Deferred: one request per import needs none, and the protocol allows more `judgment.request`s later.
- **Pass the adapter the whole environment.** Rejected: an adapter reads untrusted text; it gets the basics, what its manifest lists, and the data directory's location.

## Consequences
- Commitments from sources other than meetings reuse the adapter and the request shape; only the turns' origin changes.
- Supersedes ADR 0011 §3 and its "manifest-declared extractor plugin" rejection (see the amendment there). The check that survives it is unchanged.
- A new `Role` value and one new message are additive: protocol stays 1.x.
