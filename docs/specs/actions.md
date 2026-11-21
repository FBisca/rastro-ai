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
- THEN the issue create the fake Linear server receives holds exactly the title and description the owner accepted, unchanged, the Linear ids of the team and assignee the owner accepted, and the run's random issue id, and no other field

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

### Requirement: Owner rules live in config
Rastro SHALL read each `[[rules]]` entry in `config.toml` as an owner rule naming an action, a team and the assignee `me`, and SHALL fail any command on an invalid rule, naming the rule and its bad field.

#### Scenario: a valid rule loads
- WHEN `config.toml` holds a rule `my-issues` with action `linear.create_issue`, team `H` and assignee `me`, and the owner runs `rastro rules check`
- THEN the output lists `my-issues` with its action, team `H` and assignee `me`

#### Scenario: a rule naming an unknown action is refused
- WHEN a rule `bad` names an action no installed provider declares and the owner runs `rastro queue`
- THEN the command fails naming the rule `bad` and the field `action`, and nothing is recorded

#### Scenario: a rule whose assignee is not the owner is refused
- WHEN a rule `bad` sets `assignee = "someone"` and the owner runs `rastro rules check`
- THEN the command fails naming the rule `bad` and the field `assignee`

### Requirement: A rule's grace comes from the table
A rule's grace window SHALL be its own `grace` when set, else its action's sensitivity in `[grace]`, else the default table (`linear.create_issue` is medium, 10 minutes).

#### Scenario: the default grace
- WHEN a rule sets no `grace` and `config.toml` has no `[grace]` table
- THEN `rastro rules check` shows its grace as 10 minutes

#### Scenario: the grace table is edited
- WHEN a rule sets no `grace` and `[grace]` sets `medium = "3m"`
- THEN `rastro rules check` shows its grace as 3 minutes

#### Scenario: a rule's own grace wins
- WHEN a rule sets `grace = "1m"` and `[grace]` sets `medium = "3m"`
- THEN `rastro rules check` shows its grace as 1 minute

### Requirement: A matching rule starts an Act
When an agent's action proposal matches a rule on action, team and assignee `me`, and is not flagged as a possible duplicate, `rastro actions propose` SHALL record a pending Act run under that rule, due when its grace ends, SHALL answer with the run, the time it runs and its stop command, and SHALL send nothing.

#### Scenario: a matching proposal starts a pending Act
- WHEN with rule `my-issues` (team `H`, grace 10 minutes) an agent runs `rastro actions propose linear.create_issue --title T --team H --why W` at 10:00
- THEN the answer names a run, says it runs at 10:10 unless stopped and gives `rastro actions stop <run>`; `rastro actions` lists that run as `pending` at level `act`; the fake Linear server has received no request

#### Scenario: a proposal for another team waits in the queue
- WHEN with rule `my-issues` on team `H`, an agent proposes `linear.create_issue` for team `ENG`
- THEN `rastro queue` lists it as an action proposal, and `rastro actions` lists no run

#### Scenario: a possible duplicate waits in the queue
- WHEN with rule `my-issues` matching, an agent proposes a title equal to an open observed Linear issue's title
- THEN `rastro queue` lists the proposal with its "possible duplicate of" mark, and `rastro actions` lists no run

### Requirement: An Act runs when its grace ends
`rastro tick` SHALL execute each pending Act run whose grace has ended, creating the issue exactly once without an owner accept, and SHALL leave every pending Act whose grace has not ended untouched.

#### Scenario: tick before the grace ends sends nothing
- WHEN a pending Act is due at 10:10 and `rastro tick` runs at 10:09
- THEN the fake Linear server has received no create, and the run still reads `pending`

#### Scenario: tick after the grace ends creates the issue
- WHEN a pending Act is due at 10:10 and `rastro tick` runs at 10:10
- THEN the fake Linear server has received one create holding exactly the proposal's title, description, team, assignee and the run's issue id, and the run reads `done`

#### Scenario: a second tick creates nothing more
- WHEN a pending Act ran to `done` at 10:10 and `rastro tick` runs again at 10:11
- THEN the fake Linear server has still received one create

### Requirement: An Act is checked again before it runs
Before executing a pending Act, Rastro SHALL check that its rule still exists and matches, that no claim it cites was retracted, and that no open observed issue holds its title; when any check fails the run SHALL read `returned`, nothing SHALL be sent, and its proposal SHALL wait in the queue.

#### Scenario: the rule was removed
- WHEN the rule a pending Act started under is removed from `config.toml` and `rastro tick` runs after its grace
- THEN the fake Linear server has received no create, the run reads `returned`, and `rastro queue` lists its proposal as an action proposal

#### Scenario: a cited claim was retracted
- WHEN a claim a pending Act cites is retracted and `rastro tick` runs after its grace
- THEN the fake Linear server has received no create, and the run reads `returned`

#### Scenario: an issue with the same title appeared
- WHEN a sync observes an open Linear issue whose title equals a pending Act's title, and `rastro tick` runs after its grace
- THEN the fake Linear server has received no create, and the run reads `returned`

### Requirement: A proposal a rule acted on leaves the queue
Once the Act run a rule started on a proposal is no longer pending, the proposal SHALL leave `rastro queue` and the matches `rastro rules check` lists, and an answer to it SHALL be refused, unless the run reads `returned`.

#### Scenario: a done Act's proposal
- WHEN `rastro tick` creates the issue of a pending Act after its grace
- THEN `rastro queue` no longer lists its proposal, and `rastro queue accept` on it is refused with no second run recorded

#### Scenario: an undone Act's proposal
- WHEN the owner undoes a done Act
- THEN `rastro queue` does not list its proposal

#### Scenario: a failed Act's proposal
- WHEN a pending Act's execution fails at its grace end
- THEN `rastro queue` does not list its proposal, and `rastro rules check` lists it under no rule's matches

### Requirement: The owner stops a pending Act
`rastro actions stop <run>` SHALL record the owner's stop of a pending Act run, after which the run SHALL read `stopped` and never execute, and SHALL refuse a run that is not pending.

#### Scenario: a stopped Act never runs
- WHEN the owner runs `rastro actions stop <run>` on a pending Act and `rastro tick` runs after its grace
- THEN the fake Linear server has received no create, the run reads `stopped`, and `rastro queue` lists no item for its proposal

#### Scenario: stopping a done run is refused
- WHEN the owner runs `rastro actions stop <run>` on a run that reads `done`
- THEN the command fails naming `rastro actions undo <run>`, and nothing is recorded

#### Scenario: stop is not on MCP
- WHEN an agent lists the MCP server's tools
- THEN no tool stops a run or writes a rule

### Requirement: A pending Act runs only once
The queue SHALL refuse accepting, editing or rejecting a proposal that has a pending Act, naming the run and its stop command.

#### Scenario: accepting a pending Act's proposal is refused
- WHEN the owner runs `rastro queue accept` on the proposal of a pending Act
- THEN the command fails naming the run and `rastro actions stop <run>`, and `rastro actions` lists only that one run

### Requirement: A rule's daily cap
A rule with `daily_cap` SHALL start at most that many Act runs per local calendar day, counting stopped, returned and undone runs, and a matching proposal past the cap SHALL wait in the queue as a normal proposal.

#### Scenario: past the cap the proposal waits
- WHEN rule `my-issues` has `daily_cap = 2`, two of its Act runs started today and one of them was stopped, and an agent proposes a third matching issue
- THEN the answer says it waits in the queue because `my-issues` reached its cap of 2, `rastro queue` lists it, and `rastro actions` lists no new run

#### Scenario: the cap resets the next local day
- WHEN rule `my-issues` with `daily_cap = 2` started two Act runs on 2026-11-19 local time, and an agent proposes a matching issue on 2026-11-20 local time
- THEN the answer names a new pending Act run

### Requirement: A pending Act shows where the owner looks
`rastro queue` and the `actions` read on the CLI and MCP SHALL show each pending Act run with the time it runs and its stop command.

#### Scenario: the queue shows a pending Act first
- WHEN one pending Act due at 10:10 and one action proposal exist, and the owner runs `rastro queue` at 10:04
- THEN the first line names the pending Act's title, "runs in 6 min" and `rastro actions stop <run>`, before the action proposal

#### Scenario: agents see a pending Act over MCP
- WHEN an agent calls the MCP tool `actions` while one Act is pending
- THEN the result lists the run as `pending` with the time it runs and its stop command

### Requirement: An Act run explains itself
The `actions` read of one Act run SHALL show its level `act`, the copy of the rule it started under, its proposal with the why, and the time its grace ended, and SHALL show no owner accept.

#### Scenario: why did this Act run
- WHEN an agent reads `actions show <run>` for a done Act run
- THEN the result holds level `act`, the rule `my-issues` with its action, team, assignee, grace and cap as they were when the run started, the proposal with its why, and no accept intent

#### Scenario: the rule copy survives a config change
- WHEN the owner changes rule `my-issues` to team `ENG` after an Act run under it started
- THEN `actions show <run>` still shows the rule with team `H`

### Requirement: An Act run can be undone
`rastro actions undo <run>` SHALL undo a done Act run as it undoes an owner-accepted run.

#### Scenario: undo an Act
- WHEN the owner runs `rastro actions undo <run>` on a done Act run whose issue only the owner touched
- THEN the fake Linear server has received one delete of that issue, and the run reads `undone`

### Requirement: The owner checks what each rule does now
`rastro rules check` SHALL show, for each rule, its match, grace, cap, today's count, its pending Acts and the queued proposals it matches, on the CLI and over MCP, and SHALL record nothing but the read log row an MCP call appends.

#### Scenario: today's count and pending Acts
- WHEN rule `my-issues` with `daily_cap = 5` started two Act runs today and one of them is pending
- THEN `rastro rules check` shows `my-issues` with 2 of 5 used today and lists the pending run

#### Scenario: queued proposals the rule matches
- WHEN an action proposal for team `H` was queued before rule `my-issues` on team `H` was written
- THEN `rastro rules check` lists that proposal under `my-issues` as one it matches

#### Scenario: agents check rules over MCP
- WHEN an agent calls the MCP tool `rules_check`
- THEN the result holds the same rules, counts and pending runs as `rastro rules check --json`, and the store's intents and runs are unchanged

### Requirement: A real issue goes through a rule
The owner SHALL have one real Linear issue created by an Act under their rule, without an accept, before the slice is done.

#### Scenario: switch day
- WHEN with the launchd job calling `rastro tick` every minute and a rule on the owner's team, an in-session agent proposes the owner's next real Linear issue and the owner lets the grace window pass
- THEN the issue exists in Linear with exactly the proposed title and description

### Requirement: A run says where its proposal stands in the queue
`actions` with a run id, on the CLI and MCP, SHALL state whether the run's proposal waits in `rastro queue` and, when it does not, whether the owner's answer or a run's status took it out, agreeing with what `rastro queue` lists.

#### Scenario: a returned Act's proposal waits
- WHEN a rule's pending Act reads `returned` after its re-check fails, and the owner reads that run with `rastro actions show` and the MCP tool `actions`
- THEN both reads state that the proposal waits in the queue, and `rastro queue` lists it

#### Scenario: a done Act's proposal was taken out by its run
- WHEN a rule's Act reads `done` after `rastro tick` creates its issue, and the owner reads that run with `rastro actions show` and the MCP tool `actions`
- THEN both reads state that the proposal left the queue because that run reads `done`, and `rastro queue` does not list it

#### Scenario: an accepted proposal was taken out by the owner
- WHEN the owner accepts an action proposal with `rastro queue accept` and reads the run it started with `rastro actions show` and the MCP tool `actions`
- THEN both reads state that the proposal left the queue because of the owner's accept, naming the accept intent's id

### Requirement: Hostile proposal content does not widen Act reach
Rule matching SHALL act only on action, team and assignee, and instruction-like proposal content SHALL NOT create an Act outside the matching rule's terms.

#### Scenario: injection content creates no extra Act
- WHEN a proposal whose title and description embed instructions to create issues for another team or another action is proposed under a rule on team H
- THEN the only Act recorded is the rule's own (linear.create_issue, team H, assignee me), and no run exists for any other team or action

### Requirement: Hostile content does not bypass the Act safety gates
An Act whose proposal content is hostile SHALL remain subject to grace, stop and recheck exactly as a benign Act is.

#### Scenario: grace holds under hostile content
- WHEN an Act with injection-laden title and description is pending within its grace window and tick runs
- THEN the fake Linear server receives no request before grace ends

#### Scenario: stop holds under hostile content
- WHEN a pending hostile-content Act is stopped and later ticks run
- THEN no issue create is sent for it, ever

#### Scenario: recheck holds under hostile content
- WHEN a hostile-content Act's recheck finds an open issue with the same title during grace
- THEN the Act returns and no issue create is sent

### Requirement: The outbound payload stays exact under hostile content
The issue create for a hostile-content Act SHALL carry exactly the five input fields with the content as authored, and SHALL add no Rastro id, link, why, citation, footer, label or marker even when the content itself embeds Rastro-looking ids.

#### Scenario: the payload is exact under hostile content
- WHEN a hostile-content Act executes whose description embeds injection text, markdown and Rastro-looking ids and links
- THEN the fake Linear server receives exactly one IssueCreate whose input keys are exactly assigneeId, description, id, teamId, title, with title and description unchanged as authored and no added Rastro field

#### Scenario: hostile content drives no extra request
- WHEN the same hostile-content Act executes
- THEN the recorded operation sequence is exactly IssueLookup, Teams, Viewer, IssueCreate

#### Scenario: live proof on real Linear
- WHEN the owner runs the adversarial scenario against real Linear on a disposable issue at the release commit
- THEN the recorded run shows the exact payload and containment with no escape, and the result is recorded for the G7 gate
