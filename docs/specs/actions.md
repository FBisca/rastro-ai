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
