# Owner note, 2026-11-21: make T1 a conversation contract

Status: owner direction, given after this plan's approval. It changes T1's scope, so the plan needs re-planning and re-approval before T1 starts. The approved `plan.yaml` and `brief.md` are unchanged.

Resolved 2026-11-22: re-planned and re-approved. Turns come from the Slack extractor through `conversation.turns`, and the core checks them (brief D10–D14).

## Direction
T1 moves meetings into a shared conversation store. Make that store a **conversation contract**, not a second meeting-shaped kind in the core:

- The core defines a conversation as turns: speaker (an account or an unresolved label), text, time and what the turn replies to.
- A source plugin makes turns from its own evidence: a transcript importer for meetings and calls, the Slack connector for chat, a mail connector later. Turn-making, speaker-to-account resolution and the owner-involvement rule are deterministic and belong to that plugin, the way keys belong to base and add-on extractors (`docs/decisions/plugins.md` §54).
- The core keeps what must stay trusted: the task runner and its gates, the session boundary, the turn record, the queue and accept.

## Why
- Each new kind (email, calls, other chat tools) would otherwise add turn-making and identity code to the core, against concept §4 ("everything is a plugin").
- T1 already moves meetings into a shared store, so this is the cheapest time to define the contract.
- Conversations will later join topics (`docs/decisions/knowledge.md` §80), which needs one shape for every kind.

## Open for re-planning
- Whether `meeting.import` becomes a transcript connector or stays a core operation over the contract.
- How a source plugin declares its involvement rule, so the core can still check it (a rule the core runs, as task-template rules are, or a plugin claim the core validates).
