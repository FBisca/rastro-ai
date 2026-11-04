# Rastro: concept (north star)

*Rastro* is Portuguese for trail, trace or track. Every claim Rastro holds leaves a trail back to the evidence behind it.

This file is the product vision. Each scope's decisions live in `docs/decisions/`; terms are defined in `docs/GLOSSARY.md`. When a decision and this file disagree, this file wins and the decision is revisited.

## Why it exists
People now take in many times more information than they can process: work, the outside world, personal life, friends, family. Much of it is noise or machine-written slop, and the commitments, decisions and changes that matter are buried inside it.

Rastro takes that load off its owner in a way they can trust. It knows what is true, it can show why it believes it, it acts on the owner's behalf when it has earned that, and it asks when it hasn't.

## Outcome
**An open, pluggable personal assistant that keeps a cited understanding of a person's world, and acts, proposes or asks on their behalf, learning over time which to do.**

Success means the owner can stop checking the sources themselves.

## Who it is for
- Anyone who wants an assistant over their own information.
- Built as a product, in the open, with friends and contributors.
- Every part an owner would want to change can be changed without forking the core.

## Core ideas

### 1. Knowledge of evidence
The memory is a graph of evidence about the owner's world: people, commitments, decisions, projects, conversations, sources, and how they relate.
- **Cited.** Every claim points to the evidence it came from. Text alone is never knowledge.
- **Time-aware.** Facts start and stop being true; history is kept.
- **One identity per thing.** The same person seen in five places is one entity.
- **Honest about gaps.** "Looked and found nothing", "couldn't look" and "never looked" are different answers.
- **Correctable.** The owner can confirm, fix or retract anything.

See `decisions/knowledge.md`, `decisions/vocabulary.md` and `decisions/sharing.md`.

### 2. Act, propose, ask
Every potential action sits at one level: **Act** (done visibly, with an undo where possible), **Propose** (prepared, waiting for a yes), or **Ask** (it lacks the knowledge or the trust).
- An Act needs deterministic evidence.
- Levels are set per action and context, and they move with evidence.
- The owner can always see and override them.

See `decisions/levels.md` and `decisions/learning.md`.

### 3. Hands
An action is anything an agent can do: a workflow, a skill, a subagent, a tool call, a message, a draft, a calendar change, a code change. Owners and contributors define what actions exist and what each one may touch.

### 4. Everything is a plugin
The core is small. Everything around it plugs in:
- connectors (sources)
- extractors
- vocabularies
- workflows
- actions
- renderers
- views
- policy packs
- agent tools (Claude Code, Codex, a local model, anything else; none is special to the core)

Adding a plugin should be an afternoon's work. See `decisions/plugins.md`.

### 5. A library first
The core is something other programs build on, the way herdr's `hc` (https://herdr.dev/core/) is a core that terminals and tools build on. A CLI, an MCP server, a native app, an editor integration or someone else's agent can all drive it. macOS and Linux are required; Windows is wanted. See `decisions/surfaces.md`.

### 6. Owner-defined surfaces
How the owner sees and touches Rastro is up to them: a notes vault, a terminal, a Herdr pane, notifications, a dashboard, a native app, many at once. Interruptions are tiered and governed by the owner's rules. See `decisions/renderers.md` and `decisions/interruptions.md`.

### 7. Trust over volume
- It is judged on usefulness, not output.
- It never surfaces what is already done, known or tracked.
- It reads untrusted content without letting that content steer what it does.
- The owner controls where data lives and what leaves the machine.
- Every run and every act is recorded, so usefulness is measured, not assumed.

## What the prototype taught us
These come from `agent-fleet`, the private 2026-10 prototype, and are stated as problems, not solutions:
- **Duplicates and stale items destroy trust.** About 30% of proposed tasks were accepted; half of the drops were already done or tracked.
- **The same thing appears in many places.** Without one identity, the assistant contradicts itself.
- **A reply is not proof a commitment was met.**
- **Jobs hardcoded to one person and one employer don't transfer.**
- **Rituals with the human present carried the most value.**
- **Unused parts accumulate unless usefulness is visible.**
- **Source outages look like silence** unless the assistant says it couldn't look.

## Workflows the first version should be able to express
These are examples that prove the plugin model, not a fixed scope:
- **Find commitments:** asks, promises, dates, waiting-on.
- **Check against what is known** before showing anything.
- **Morning define:** priorities, approvals, what fits the calendar.
- **What changed** in followed channels, pages and projects.
- **News radar.**
- **Capture** a session or meeting.
- **Project updates** in the owner's voice, published only on approval.
- **Weekly review.**
- **Find agent work a plain rule could do.**

## Reference material
- Prototype: `agent-fleet` (private) at `433a686`.
- Knowledge-graph research (2026-11): `docs/research/knowledge-graphs-2026-11.md`.
- Related work:
  - mycelium (https://github.com/mycelium-ai-labs/mycelium): an evidence/justification graph with human ratification.
  - agent-runtime (https://github.com/FBisca/agent-runtime): agent delegation over a durable local store.
