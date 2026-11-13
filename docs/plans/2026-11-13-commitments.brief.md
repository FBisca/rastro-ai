# Brief: Commitments from conversations

Slug: `commitments`. Contract: `plan.yaml`. Lane: Full — the slice is regular · complex, carries
security, external, state and migration risks, and has five agent tickets plus the owner's.

## The contract

What the implementation and the oracle both hold to.

**The read.** `rastro commitments [--source <name>] [--term <term>]` and the MCP tool
`commitments`, with the arguments `{source?: string, term?: string}`. The result is the v2
envelope around one object:

```json
{
  "notice": "Hypotheses an adapter proposed from a conversation. They are not commitments Rastro knows: it says nothing about whether they are open, done or tracked.",
  "sources": [
    {
      "kind": "meeting",
      "name": "2026-10-23-rbac",
      "scope": "personal",
      "extraction": {
        "state": "succeeded", "cause": null, "reason": null,
        "run": "12", "finished_at": "2026-11-13T19:00:00Z",
        "adapter": "test-adapter", "replay_of": null, "excluded": 0
      },
      "coverage": { "state": "found", "turns_read": 6, "turns_total": 6, "reason": null },
      "stale": false,
      "results_from": { "run": "12", "imported_at": "2026-11-13T19:00:00Z", "replayed_at": null },
      "hypotheses": [
        {
          "status": "hypothesis",
          "kind": "promise",
          "responsible": { "state": "mapped", "label": "Alice", "identity": { "id": "e_…", "identifier": "rastro:person/…" } },
          "recipient": { "state": "mapped", "label": "Bob", "identity": { "id": "e_…", "identifier": "rastro:person/…" } },
          "promise": "send Bob the budget",
          "stated_date": { "state": "stated", "text": "by Friday" },
          "quote": { "text": "I'll send Bob the budget by Friday", "turn": 1, "speaker": "Alice", "start": 0, "end": 37 },
          "terms": ["std:owes", "std:owed_to", "std:due"]
        }
      ]
    }
  ]
}
```

- A source with no hypotheses is still listed, with `hypotheses: []` and the coverage state that
  says why (`found` with nothing, `empty`, or `couldn't look` with its reason).
- A filter that matches nothing returns `sources: []`.
- The `kind` is one of `ask`, `promise`, `waiting_on`. The `terms` follow the fields: `std:owes`
  when the hypothesis has a responsible party, `std:owed_to` when it has a recipient, `std:due`
  when it states a date, `std:waiting_on` when it is a waiting-on.
- A party is `{state: "mapped" | "unknown", label, identity}`; `identity` is there only when the
  owner mapped it explicitly, and a party that is only named carries its label and no identity.
- The list never says a commitment is open, done or tracked.

**The adapter.** A plugin of kind `agent-tool-adapter`, bound in the config:

```toml
[commitment_adapter]
plugin = "test-adapter"
timeout_seconds = 120
```

Its manifest declares the kind and the agent-tool capabilities it needs, never a named tool:

```toml
id = "test-adapter"
name = "Rastro test agent tool adapter"
version = "0.1.0"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "adapter"
agent_tool = { capabilities = ["judge_text"] }

[permissions]
submit = []
scopes = []
env = []
actions = []
```

The core speaks the plugin protocol with it:

1. `initialize` — params `{protocol, manifest_id, role: "agent_tool_adapter"}`; the adapter
   answers `{protocol, manifest_id}`.
2. `judgment.request` — params `{instructions, owner_speaker, turns: [{turn, speaker, text}],
   fields}`; the adapter answers `{candidates: [{turn, kind, responsible, recipient, promise,
   stated_date, quote}]}`, where `responsible` and `recipient` are speaker labels and
   `stated_date` is the date as the quote states it. `kind` is `ask`, `promise` or `waiting_on`.
3. The core checks every candidate itself: the quote is an exact contiguous piece of the named
   turn, the owner is involved, and a stated date is a piece of the quote. A candidate that
   fails is excluded and counted; a response that breaks the shape fails the run.

## What changes
- `rastro commitments` / MCP `commitments`: no such list → one source-neutral list over every
  source that has hypotheses, each with its source, kind, promise, stated date, parties, exact
  quote, terms, `hypothesis` status, and the source's extraction, coverage and stale state (T1)
- Filters: none → `--source` and `--term`, where a filter that matches nothing is an empty list,
  and a scope that hides a source hides its hypotheses without failing the read (T1)
- The MCP tool set: five tools → six, with `commitments` beside `search`, `coverage`, `about`,
  `why` and `meeting_commitments`; the meeting read stays until the next schema version (T1)
- The standard vocabulary: none → a bundled `std:` namespace with `std:Commitment`, `std:owes`,
  `std:owed_to`, `std:due` and `std:waiting_on`, each with the fields a vocabulary entry needs,
  and every hypothesis carries the terms it expresses (T2)
- The extractor that proposes candidates: a configured one-shot command → an agent tool adapter,
  a plugin of kind `agent-tool-adapter` that declares the agent-tool capabilities it needs, speaks
  the plugin protocol with a judgment message, and whose response the core validates strictly
  before it keeps any candidate (T3)
- The reference adapter: none → `rastro-adapter-claude-code`, one adapter that lets Claude Code do
  the judgment; the config binds it by plugin id (T3)
- The recording: per meeting → behind the source-neutral seam, so any source's interpretation is
  recorded with its spans, replayed with no model call, and never re-called at a rebuild (T4)
- Bot-authored candidates: kept → excluded and counted when the evidence names the mapped account
  as a bot; a party only named is still kept (T5)
- Open question #6 (filtering bots for usefulness): open → decided (T5)

## Decisions
- D1 One source-neutral `commitments` list on CLI and MCP, with source and term filters, added
  within v2: the roadmap names that list, and a hypothesis cannot assert "open", so the
  concept-level `open_commitments(person)` is reconciled to it; the meeting read stays until the
  next schema version, because removing an operation is not additive (roadmap slice 5; surfaces
  §5, §33, §48–49)
- D2 Hypotheses stay outside the claims projection, which stays observed-only: accepting them is
  slice 6's decision (open question #12), and the projection's trust model is not slice 5's to
  change (knowledge §23, §48; ADR 0008, ADR 0009, ADR 0011)
- D3 The judgment path generalizes, not the deterministic one: the core refuses a claim resting on
  unstructured content, so the commitment path is the one that becomes an agent tool adapter — a
  plugin kind declaring the capabilities it needs from an agent tool, never a named tool, bound by
  plugin id, with its response validated strictly by the core (plugins §2, §4, §10, §12; ADR 0003,
  ADR 0008, ADR 0011; glossary)
- D4 The minimal standard vocabulary ships bundled with the core rather than as a plugin the owner
  installs: it is needed by every source, and plugin installation is slice 15's work (vocabulary
  §1, §5; glossary)
- D5 The recorded interpretation is recorded and replayed with no model call, a rebuild never calls
  a model, and the recorded output stays out of exports: what becomes precious is the owner's
  *confirmed* interpretation, which slice 6 decides and slice 13 exports (knowledge §46, §53, §12;
  meetings §12; sharing §8)
- D6 A candidate mapped to an account the evidence says is a bot is excluded and counted; a party
  only named is not: filtering needs evidence, never a guess (open question #6; knowledge §27)
- D7 A hypothesis never backs an Act and the slice introduces no Rule: untrusted content never
  raises a level (levels §2, §9)
- D8 The switch day: the owner reads a real conversation's commitments over MCP, from the list,
  without opening the transcript by hand (roadmap slice 5's outcome)

## Out of scope
- Commitments from GitHub and Linear comments, Slack or mail: the next sources, once this list
  proves useful (parking lot).
- Accepting a hypothesis into a claim, the queue and its outcomes: slice 6.
- Removing the meeting read (`meeting.commitments`, the MCP tool `meeting_commitments`): the next
  schema version, since removal is not additive within v2.
- An agent tool adapter for a runtime other than Claude Code: the kind is the plug point; the owner
  writes their own or asks for one.
- Plugin install and permission diffs: slice 15. The adapter is installed by hand, as the
  connectors are today.

## Risks
- security: the adapter runs outside the core and reads untrusted text — the core validates its
  response strictly and keeps only what its own checks accept (S6); a plugin that reads untrusted
  content holds no action permissions (S7).
- external: the reference adapter shells out to a runtime — it is a plugin like any other, and its
  output is data the core checks, never instructions.
- state, migration: the list and the recording move behind the source-neutral seam — the recorded
  output, the spans and the hypotheses stay per source, and a rebuild never calls a model (S8).
