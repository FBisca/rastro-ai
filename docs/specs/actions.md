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

### Requirement: Action providers are a plugin role
The host SHALL load a plugin of kind `action_provider` that declares its actions and no evidence permission, SHALL refuse any other plugin that declares actions, and SHALL offer only the actions installed providers declare.

#### Scenario: an action provider loads
- WHEN an `action_provider` manifest declares the action `linear.create_issue` and no `submit` permission
- THEN the host loads it and `rastro actions propose linear.create_issue` is accepted

#### Scenario: a provider that also submits evidence is refused
- WHEN an `action_provider` manifest declares an action and a `submit` permission
- THEN the host refuses the plugin naming it and the `submit` permission, and no action is offered

#### Scenario: an action no provider declares is refused
- WHEN an agent proposes an action that no installed provider declares
- THEN the command fails naming the action, and the queue holds no new item

### Requirement: Each action declares its reach and reversibility
An action provider's manifest SHALL declare, for each action, its reach and its reversibility, and the host SHALL refuse the plugin when either is missing.

#### Scenario: an action without its risk is refused
- WHEN an `action_provider` manifest declares an action with no reversibility
- THEN the host refuses the plugin naming the action and `reversibility`, and no action is offered

### Requirement: An agent proposes an action
`rastro actions propose` SHALL record one pending action proposal bound to the source it writes to and that source's scope, carrying the action, its fields, the proposer's why and the cited claims, SHALL list it in the queue, and SHALL run nothing.

#### Scenario: a proposal waits in the queue
- WHEN an agent runs `rastro actions propose linear.create_issue --title T --why W` with `default_team` set
- THEN `rastro queue` lists one action proposal with a content-addressed id naming the action, the title, the default team, the owner as assignee, the why and the Linear source with its scope, and the fake Linear server has received no request

#### Scenario: a proposal without a why is refused
- WHEN an agent proposes an action without `--why`
- THEN the command fails naming `--why`, and nothing is recorded

#### Scenario: a citation the source's scope cannot see is refused
- WHEN an agent proposes a work-scoped Linear action with `--cite` naming a claim visible only in a personal scope
- THEN the command fails naming the claim id and the scope `work`, and nothing is recorded

### Requirement: A proposal names its team
An action proposal for `linear.create_issue` SHALL take its team from `--team`, else from `[actions.linear] default_team`, and SHALL be refused when neither names one.

#### Scenario: no team anywhere is refused
- WHEN an agent proposes `linear.create_issue` without `--team` and the config has no `default_team`
- THEN the command fails naming `--team` and `[actions.linear] default_team`, and nothing is recorded

### Requirement: A duplicate title is flagged
An action proposal for `linear.create_issue` SHALL name each open observed Linear issue visible in its scope whose title equals its own exactly, and SHALL stay in the queue.

#### Scenario: an open issue with the same title is named
- WHEN an observed Linear issue in an open state has exactly the proposal's title
- THEN the queue item reads "possible duplicate of" that issue's identifier, and accepting it is still allowed

#### Scenario: a closed issue with the same title is not
- WHEN the only observed Linear issue with the proposal's title is Done or Canceled
- THEN the queue item carries no duplicate mark

### Requirement: Accepting a proposal runs the action once
`rastro queue accept` on an action proposal SHALL record the accept intent and one Action run holding a random issue id in a single transaction, and the run SHALL create at most one issue however often it is executed.

#### Scenario: an accepted proposal creates the issue
- WHEN the owner accepts an action proposal
- THEN the fake Linear server receives exactly one issue create with the proposal's title, team and assignee, and `rastro actions` lists the run `done` with the issue's identifier and URL

#### Scenario: a repeated execution creates nothing new
- WHEN the same Action run is executed a second time after its issue was created
- THEN the fake Linear server still holds one issue for the run, and the run reads `done` once

### Requirement: Nothing about Rastro reaches a source
The request an action sends to an outside system SHALL carry only the content written for that system, and never a Rastro id, link, why, citation, footer, label or marker.

#### Scenario: the Linear payload is exactly the issue
- WHEN an accepted proposal with a why and two cited claims creates its issue
- THEN the issue create the fake Linear server receives holds exactly the title, description, team and assignee the owner accepted, each unchanged, plus the run's random issue id, and no other field

### Requirement: An interrupted run is finished by tick
`rastro tick` SHALL execute each accepted Action run that is neither done nor failed, SHALL look the run's issue id up before creating it, and SHALL let only one process execute a run at a time.

#### Scenario: a run cut off before sending is executed
- WHEN an accepted run is recorded and the process stops before any request is sent, then tick runs
- THEN the fake Linear server receives one issue create for the run, and the run reads `done`

#### Scenario: a lost response is reconciled
- WHEN the fake Linear server created the run's issue but the response never arrived, then tick runs
- THEN tick finds the issue by the run's id, sends no second create, and the run reads `done` with its identifier

#### Scenario: two executors make one issue
- WHEN an accept and a tick execute the same run at the same time
- THEN the fake Linear server receives one issue create, and the other executor leaves the run untouched

### Requirement: The owner edits or rejects an action proposal
`rastro queue edit` on an action proposal SHALL accept it with the owner's changes to its title, description, team or assignee, as edit accepts a commitment proposal, and `rastro queue reject` SHALL close it with no run.

#### Scenario: an edited proposal runs as edited
- WHEN the owner edits an action proposal's title
- THEN the fake Linear server receives one issue create with the edited title, the run reads `done`, and the queue no longer lists the item

#### Scenario: a rejected proposal runs nothing
- WHEN the owner rejects an action proposal
- THEN the fake Linear server receives no request, and `rastro actions` lists no run for it

### Requirement: The owner reads why an action ran
`actions` SHALL be a catalog read on the CLI and MCP that returns each run with its status, and for one run its proposal, why, cited claims with their evidence, accept intent, level, cell, reach and reversibility, visible only in the scope of the source the run writes to.

#### Scenario: a run explains itself
- WHEN an agent calls the MCP tool `actions` with a run id after the owner accepted a cited proposal
- THEN the result holds the action, its fields, the why, each cited claim with its evidence, the accept intent's id and time, level `propose`, the cell (action and team), reach `others see it`, reversibility `can be undone` and status `done`

#### Scenario: another scope sees neither proposal nor run
- WHEN an MCP server launched with a scope other than the Linear source's reads `queue` and `actions`
- THEN the action proposal and its run are absent, and both reads still succeed

### Requirement: The owner undoes an action
`rastro actions undo <run>` SHALL delete the issue a done run created and record an undo intent, and SHALL refuse when someone other than the owner commented on or edited the issue since it was created.

#### Scenario: an untouched issue is deleted
- WHEN the owner undoes a done run whose issue nobody else has changed
- THEN the fake Linear server receives one delete for that issue, and `rastro actions` lists the run `undone` with the undo intent's id and time

#### Scenario: a colleague's comment blocks undo
- WHEN the owner undoes a done run whose issue another user has commented on
- THEN the command fails naming that user and the comment, no delete is sent, and nothing is recorded

#### Scenario: a colleague's edit blocks undo
- WHEN the owner undoes a done run whose issue another user has edited
- THEN the command fails naming that user and the edit, no delete is sent, and nothing is recorded

### Requirement: Undo deletes only what it can verify
`rastro actions undo` SHALL read the issue's full comment and change history before deleting, and SHALL refuse when that history cannot be read whole or the run is not done.

#### Scenario: a run that is not done cannot be undone
- WHEN the owner undoes a run that is `failed` or already `undone`
- THEN the command fails naming the run's status, and nothing is recorded

#### Scenario: unreadable history refuses
- WHEN the fake Linear server fails the issue's history read on any page
- THEN the command fails saying the history could not be checked, no delete is sent, and nothing is recorded

#### Scenario: the owner's own changes do not block
- WHEN the owner undoes a done run whose issue only the owner has edited and commented on
- THEN the fake Linear server receives one delete, and the run reads `undone`

### Requirement: Undo is safe to repeat
`rastro actions undo` SHALL record the run `undone` when its issue is already deleted, without sending a second delete.

#### Scenario: a lost delete response
- WHEN the delete reached the fake Linear server but its response was lost, and the owner runs undo again
- THEN the command succeeds without a second delete, and the run reads `undone` with one undo intent

### Requirement: One tick syncs every stale source
`rastro tick` SHALL sync each source whose last complete check is older than its freshness rule, skip the fresh ones, and exit at once when another tick holds the lock.

#### Scenario: only stale sources sync
- WHEN tick runs with one source past its freshness rule and one within it
- THEN one new run is recorded for the stale source and none for the fresh one

#### Scenario: a concurrent tick does nothing
- WHEN a second tick starts while the first still holds the tick lock
- THEN the second exits 0 saying a tick is already running, and records no run

#### Scenario: one failed source does not stop the others
- WHEN tick syncs two stale sources and the first one's sync fails
- THEN the second source still syncs, and tick exits non-zero naming the failed source

### Requirement: A failed run is retried by tick, then left failed
An Action run whose execution fails SHALL read `failed` with its error, and `rastro tick` SHALL retry it until it has made 3 attempts, then leave it failed.

#### Scenario: a failed create is recorded
- WHEN the fake Linear server returns an error to the issue create of an accepted proposal
- THEN the accept command reports the error and exits non-zero, the accept intent stays recorded, and `rastro actions` lists the run `failed` with the error and 1 attempt

#### Scenario: tick retries a failed run
- WHEN tick runs over a failed run with fewer than 3 attempts and the fake Linear server now succeeds
- THEN the server holds one issue for the run, and the run reads `done`

#### Scenario: three attempts are the end
- WHEN tick runs over a failed run that has made 3 attempts
- THEN the fake Linear server receives no request, and the run stays `failed` with 3 attempts

### Requirement: A real issue goes through Rastro
The owner SHALL have one real Linear issue created through a Rastro proposal from an in-session agent before the slice is done.

#### Scenario: switch day
- WHEN an in-session agent proposes the owner's next real Linear issue and the owner accepts it
- THEN the issue exists in Linear with exactly the accepted title and description
