# Learning: trust per cell

Paths: crates/rastro-core/src/trust/, crates/rastro-core/src/actions/, crates/rastro-core/src/queue/, crates/rastro-core/src/tick.rs, crates/rastro-core/src/read_only.rs, crates/rastro-core/src/lib.rs, crates/rastro-operations/, crates/rastro-cli/src/, crates/rastro-cli/tests/trust.rs, crates/rastro-cli/tests/trust_demotion.rs, crates/rastro-cli/tests/trust_ask.rs, crates/rastro-cli/tests/trust_usefulness.rs, crates/rastro-cli/tests/queue.rs, crates/rastro-storage-sqlite/migrations/, crates/rastro-storage-sqlite/src/, crates/rastro-cli/tests/mcp_read_log.rs, crates/rastro-cli/tests/mcp.rs, schema/

Trust answers "should it act alone?" per cell — one action × team pair — and is derived from outcomes, never stored (`../decisions/learning.md` §1, §5; `../decisions/levels.md`). A cell's level and the evidence behind it are computed at read from the runs and owner intents the visible scopes hold, under the configurable thresholds of `[learning]`; promotion is always proposed to the owner, never self-applied, and demotion is automatic and visible (`../decisions/learning.md` §6–7, §14). The trust view is a core read like the queue, defined once in the operation catalog (`../adr/0007-typed-operation-catalog.md`) and offered on the CLI and, deferred, through the MCP catalog gateway. A machine read is audited, never mistaken for the owner having seen an item.

This file holds the trust-view, demotion, decay, Ask-answer, usefulness and cost, and read-logging requirements of the learned-trust slice; the slice's remaining requirements (pin and reset, the journal read) land with their tickets.

## Requirements

### Requirement: A cell's trust is derived from its outcomes
A cell's level and evidence counts SHALL be derived at read from its runs, owner intents and shown reports, under the configurable thresholds of `[learning]`, and a rebuild SHALL reproduce the trust view and journal with no model call.

#### Scenario: clean outcomes make a cell eligible
- WHEN a cell has 10 clean accepted outcomes spanning 8 days with 0 undos in its last 20
- THEN rastro trust names the cell eligible and shows the counts behind it

#### Scenario: thresholds come from config
- WHEN config.toml sets [learning] clean_min to 5 and a cell reaches 5 clean outcomes over 7 days with 0 undos
- THEN rastro trust names the cell eligible under the configured threshold

#### Scenario: a rebuild reproduces trust
- WHEN the store is rebuilt from the same evidence, intents and shown reports
- THEN rastro trust and rastro trust journal answer exactly as before the rebuild

### Requirement: Promotion is proposed, never self-applied
An eligible cell SHALL appear in rastro trust with the exact rule text or reset command that raises it, and no cell SHALL reach Act without the owner's rule or reset.

#### Scenario: an eligible cell without a rule is proposed one
- WHEN a cell with no rule reaches eligibility
- THEN rastro trust shows a [[rules]] entry naming the action and team, and the cell's level stays Propose

#### Scenario: the owner's rule raises the cell
- WHEN the owner adds the proposed rule to config.toml
- THEN rastro trust shows the cell at Act

#### Scenario: a demoted cell is proposed a reset
- WHEN a demoted cell with a rule reaches eligibility again
- THEN rastro trust shows rastro trust reset for the cell, and its level stays demoted until the owner resets it

### Requirement: The trust view tells the whole story
rastro trust SHALL show each visible cell's level, evidence counts, last change and what the next promotion needs, computed from visible scopes only.

#### Scenario: a row tells the whole story
- WHEN a cell has outcomes and a level change
- THEN its row shows the level, the clean and undo counts, the last change with its cause, and what the next promotion needs

#### Scenario: hidden scopes count nowhere
- WHEN a cell's team lives in a scope the caller cannot see
- THEN rastro trust under the visible scopes omits the cell and counts none of its outcomes

#### Scenario: an empty store lists no cells
- WHEN no runs exist
- THEN rastro trust lists no cells and says the store holds no trust history

### Requirement: Demotion is automatic and fast
One undo of a run SHALL drop its cell one level, two or more undos in its last 20 outcomes SHALL drop it to Ask, and a retract or edit landing on a claim a run cited SHALL count as an undo.

#### Scenario: one undo drops a level
- WHEN a cell at Act has one done run undone
- THEN rastro trust shows the cell at Propose and the journal names the undo as the cause

#### Scenario: repeated undos drop to Ask
- WHEN a cell's last 20 outcomes hold two undos
- THEN rastro trust shows the cell at Ask

#### Scenario: a cited claim's correction demotes
- WHEN a claim a done run cited is retracted
- THEN the run's cell drops one level exactly as for an undo, and the journal names the retraction

### Requirement: Stopped and returned runs demote nothing
A stopped or returned run SHALL leave its cell's level unchanged.

#### Scenario: stop is not a demotion
- WHEN a pending Act is stopped and another is returned by the pre-run recheck
- THEN rastro trust shows the cell's level unchanged and the journal records no demotion for either run

### Requirement: Unused trust decays on next use
A cell unused for the configured decay window, default 30 days, SHALL drop one level the next time it is used.

#### Scenario: decay applies at next use
- WHEN a cell at Act last ran 31 days ago and a new proposal uses it
- THEN the cell reads Propose and the journal names the decay with its window

### Requirement: An Ask cell asks instead of proposing
A match whose cell sits at Ask SHALL surface as a queue question naming the action, SHALL start no run, and SHALL run the action once when the owner answers yes or reject it when the owner answers no.

#### Scenario: an Ask match becomes a question
- WHEN a rule matches a proposal whose cell is at Ask
- THEN rastro queue lists a question naming the action and title, and rastro actions lists no run

#### Scenario: answering yes runs once
- WHEN the owner answers the Ask question yes
- THEN the action runs exactly once as an accepted proposal would, and the fake Linear server receives one create

#### Scenario: answering no rejects
- WHEN the owner answers the Ask question no
- THEN nothing runs and the question's outcome reads rejected

### Requirement: Usefulness is visible per plugin
rastro trust SHALL show per-plugin engagement as acted on over seen from terminal shown reports with the time to action, reading unmeasured where no shown reports exist.

#### Scenario: engagement is measured
- WHEN a plugin's items were shown 10 times by terminal reads and acted on 6
- THEN its engagement reads 6 of 10 with the time to action from the outcomes' waits

#### Scenario: unseen is unmeasured
- WHEN a plugin has runs but no shown reports
- THEN its engagement reads unmeasured, never 0

### Requirement: Cost is wall time plus adapter turns
A plugin's cost SHALL be the wall time of its runs plus the adapter turns they used, and cost per accepted item SHALL read a dash when nothing was accepted.

#### Scenario: cost and cost per accepted
- WHEN a plugin ran 120 seconds over 4 turns and 2 of its items were accepted
- THEN its cost reads 120 seconds and 4 turns, and cost per accepted reads 60 seconds and 2 turns

#### Scenario: zero accepts divides nothing
- WHEN a plugin's runs accepted nothing
- THEN its cost per accepted reads a dash, not a division by zero

### Requirement: MCP reads are logged, never seen
The MCP server SHALL log each read call with its tool, arguments and time, and the log SHALL never write a shown report.

#### Scenario: an MCP read is logged
- WHEN an MCP client calls the queue tool
- THEN a read log row names the tool, arguments and time, and no shown report is written

#### Scenario: a terminal read is not an MCP read
- WHEN the same queue read runs on the terminal
- THEN a shown report is written and no read log row is
