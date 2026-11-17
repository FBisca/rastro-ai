# Brief: workspace specs

Slug: `workspace-specs`. Contract: `plan.yaml`. Lane: Plan, big × complex: every spec and test file changes, and the spec format is the contract every plan writes to.

## What changes

- CI: no link between specs and tests → a spec gate holds every titled requirement to a `// spec:` tag and every tag to a title, across every `.rs` file (src unit tests included) and every spec under `docs/specs/`, subfolders included (T1)
- `.orchestrate/config.yaml`: checks only → `plans: {dir: docs/plans}` and `archive: {mode: promote}` (T1)
- `CONTRIBUTING.md`: none → how the repo is built, setup, the commands, pointers to `docs/development.md` and AGENTS.md (T1)
- `docs/specs/claims.md` (T2), `evidence.md` (T3), `identity.md` (T4), `surfaces.md` (T5), `commitments.md` and `meetings.md` (T6): untitled R-bullets and hand-kept Coverage tables → titled SHALL requirements with `#### Scenario:` bullets, their proving tests tagged
- The spec gate: titled requirements only → also refuses an `- **R<n>**` bullet or a `## Coverage` table (T7)

## Decisions

D1 Specs hold `### Requirement: <Title>` blocks (one SHALL sentence, `#### Scenario:` WHEN/THEN), over the untitled `- **R<n>** WHEN…, THEN…` bullets: the plan/2 engine reads and writes only titled blocks (agent-runtime `docs/specs/python-workspace-contracts.md`; start-planning `plan-yaml.md`).
D2 `.orchestrate/config.yaml` gains `plans: {dir: docs/plans}` and `archive: {mode: promote}`, over each plan naming its own mode: the engine discovers plans and defaults the archive from the repo config (agent-runtime `.orchestrate/config.yaml`).
D3 The archive creates a spec file that does not exist yet, so `docs/specs/workspace/specs.md` needs no seeding ticket (orchestrator `spec_blocks.py:31-35`, `driver/archive.py:427-437`).
D4 Every existing spec migrates: each of the 249 R-bullets becomes a titled SHALL requirement, its tests carry tags taken from the Coverage tables, and the tables go, over titling only new requirements: an untitled spec leaves the gate vacuous and the hand-kept tables drifting (owner, 2026-11-17).
D5 The gate is agent-runtime's `scripts/spec_coverage.py` copied with three fixes (every `.rs` file incl. `src/` `#[cfg(test)]` modules, `target/` skipped, specs read recursively), over a Rust check crate or bash: no tooling crate in the product workspace, Python 3 is on every runner (owner, 2026-11-17).
D6 A tag is `// spec: <Title>` on its own line above `#[test]`; a test may carry several, over an attribute macro: no proc-macro crate, and the script already reads the comment (owner, 2026-11-17).
D7 The six product specs stay one file each; a spec becomes a folder only when it outgrows one file, and the gate reads `docs/specs/**/*.md` (owner, 2026-11-17).
D8 The gate's legacy-format check lands after every migration (expand–contract), so CI stays green while the specs move (planning, 2026-11-17).
D9 CONTRIBUTING.md is short and for people, linking `docs/development.md` and AGENTS.md instead of restating them (owner, 2026-11-17).

## Out of scope

- `docs/specs/queue.md` and the commitments-queue plan: already titled; whichever lands second tags its tests.
