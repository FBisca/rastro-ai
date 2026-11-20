# Actions

Paths: crates/rastro-core/src/actions/, crates/rastro-core/src/rules/, crates/rastro-core/src/tick.rs, crates/rastro-core/src/queue/, crates/rastro-action-linear/, crates/rastro-storage-sqlite/src/, crates/rastro-operations/, crates/rastro-cli/src/, schema/

Rastro acts in an outside system for the owner. An agent in any session proposes an action (today, `linear.create_issue`); the owner accepts it from the queue, or a rule the owner wrote in `config.toml` starts it as an Act after a grace window the owner can stop. A run executes exactly once through an `action_provider` plugin (`../adr/0015-action-provider-role.md`), keeps its why, citations and rule inside Rastro, sends the outside system only the content written for it, answers why it ran with `actions show`, and can be undone. One idempotent `rastro tick`, called by launchd every minute, runs due Acts, retries failed runs and keeps sources fresh (`../decisions/levels.md`).

## Switch day

### First action, proposed (slice 8a) — 2026-11-20

The owner installed the build from `main`, kept `[actions.linear] default_team = "H"`, and switched the
launchd job `ai.rastro.tick` to call `rastro tick`. The first real create failed: Linear answered
`INVALID_INPUT`, because the provider sent the team key `H` and `me` where Linear wants ids. The fix
landed the same day (`A created issue names Linear's own ids`, below). An in-session agent then proposed
"[mock] Rastro switch-day test 4, delete me" on team `H`; the owner accepted it from the queue and Linear
created H-10329 with exactly the accepted title and description. `actions show` gave level `propose`, the
why and the accept intent. The owner undid it with `rastro actions undo`, which moved H-10329 to Linear's
trash.

### First Act (slice 8b) — 2026-11-20

The launchd job's interval dropped from 15 minutes to 60 seconds, and the owner wrote the rule
`my-issues` (`linear.create_issue`, team `H`, assignee `me`, `daily_cap = 5`, the default 10-minute grace);
`rastro rules check` listed it. A first proposal started a pending Act shown first in `rastro queue` with
its time left; the owner stopped it with `rastro actions stop` and nothing was sent. After the id fix, an
in-session agent proposed "[mock] Rastro switch-day test 3, delete me" on team `H`; the rule matched, no
one accepted it, the grace window passed, and tick created H-10328 with exactly the proposed title and
description. `actions show` gave level `act`, the rule as it was and no accept. The owner undid it, which
moved H-10328 to Linear's trash.

### Requirement: A created issue names Linear's own ids
The Linear action provider SHALL send an issue create with the Linear id of the team the run's team key names and, for the assignee `me`, the Linear user id of the key's owner.

#### Scenario: the team key is sent as its team's id
- WHEN a run with team `H` executes against a fake Linear server that refuses a team that is not an id
- THEN the issue create the fake server receives carries team H's Linear id as its team

#### Scenario: the assignee me is sent as the owner's id
- WHEN a run with assignee `me` executes against a fake Linear server that refuses an assignee that is not an id
- THEN the issue create the fake server receives carries the owner's Linear user id as its assignee

#### Scenario: an unknown team key creates nothing
- WHEN a run's team key names no team the fake Linear server holds
- THEN the fake server receives no issue create, and the run's error names the team key
