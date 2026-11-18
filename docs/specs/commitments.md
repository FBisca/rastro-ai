# Commitments
Paths: crates/rastro-core/src/commitments/, crates/rastro-core/src/meeting.rs, crates/rastro-core/src/read_only.rs, crates/rastro-storage-sqlite/src/commitments.rs, crates/rastro-storage-sqlite/src/meeting.rs, crates/rastro-operations/, crates/rastro-cli/src/, crates/rastro-cli/tests/commitments_oracle.rs, schema/

The commitments list: the commitment hypotheses of every source that has any, in one source-neutral shape. A hypothesis is what an agent tool adapter proposed from a conversation and the core checked; it is not a claim, and nothing here says whether one is open, done or tracked. Product meaning: `../decisions/surfaces.md` §51. Architecture: `../adr/0011-meeting-commitment-hypotheses.md`, `../adr/0012-agent-tool-adapter.md`.

## The list

`rastro commitments [--source <name>] [--term <term>]` and the MCP tool `commitments` (`{source?, term?}`) return the v2 envelope around `{notice, sources}`. Each source is `{kind, name, scope, extraction, coverage, stale, results_from, hypotheses}`:

- `kind` is `meeting` (the only kind so far), `name` is the meeting's identifier.
- `extraction` is the latest run: `state` (`succeeded` or `failed`), `cause` and `reason` (null unless it failed), `run`, `finished_at`, `adapter` (the plugin id), `replay_of` and `excluded`. Run ids are strings.
- `coverage` is `found` (turns read of turns total) when the latest run looked, whether or not it found anything, and `couldn't look` with its `reason` when it failed.
- `stale` is true when the hypotheses are older than the latest import (it failed, or the latest run only replayed an older record). `results_from` is null when no run has succeeded, otherwise `{run, imported_at, replayed_at}`: the last good run, when the import it traces to finished, and when a replay last checked it.
- A hypothesis is `{status: "hypothesis", kind, responsible, recipient, promise, stated_date, quote, terms}`. `kind` is `ask`, `promise` or `waiting_on`. A party is `{state: "mapped" | "unknown", label, identity}`; `identity` is `{id, identifier}` only when the owner mapped the label explicitly, and null otherwise. `stated_date` is `{state: "stated", text}` or `{state: "unknown"}`, as the quote says it. `quote` is `{text, turn, speaker, start, end}` with the byte range inside the turn.
- `terms` are the standard vocabulary terms the hypothesis expresses: `std:owes` when it has a responsible party and is not a waiting-on, `std:owed_to` when it has a recipient, `std:due` when it states a date and `std:waiting_on` when it is a waiting-on.

A hypothesis keeps the kind its run's recorded output holds; one whose recorded output has none (recorded before adapters proposed kinds) reads as a `promise`.

## Switch day

### RBAC Q4 kickoff — 2026-10-23

The owner imported the real meeting transcript (302 speaker-labelled turns from the Teams export:
Felippe Bisca, Nathan Beale, Giuseppe Pirrelli, Carlo Elefanti) with the reference Claude Code
adapter, then read the list over MCP.

The run succeeded: coverage found 302 of 302 turns, two candidates were excluded, and thirteen
hypotheses came back, each with its kind, its promise, the exact quote it rests on and the
standard vocabulary terms it expresses. The first import failed with `invalid_quote`: a model's
quote is not always copied character for character, and the core refuses a run whose quote is not
an exact piece of the turn. The instructions now say so, and the run went through.

What the owner reads over MCP is a list of what the meeting proposed — "Felippe is waiting on
Giuseppe to finish the final migration", "Keep Felippe updated on the migration progress" — each
with the words it rests on and the turn they are in, and nothing about whether any of them is open,
done or tracked.

## Requirements

### Requirement: The list shows every hypothesis with its source and state
The `commitments` list SHALL show every hypothesis of every source with that source's kind, name and scope, the source's extraction state, its coverage state and whether its results are stale, and each hypothesis with its kind, its promise, its stated date, its responsible and recipient parties, the exact quote with its turn, speaker and byte range, the status `hypothesis` and the standard vocabulary terms it expresses.

#### Scenario: a source's hypotheses come back with their state
- WHEN the owner asks for commitments and a source has hypotheses
- THEN the list shows every hypothesis with its source (kind, name, scope), its kind, the promise text, the stated date or null, the responsible and the recipient (the label, and the entity only when the owner mapped it explicitly, else null), the exact quote with its turn, speaker and byte range, the status `hypothesis` and the standard vocabulary terms it expresses, and for each source its extraction state, its coverage state and whether its results are stale

#### Scenario: a record without a kind reads as a promise
- WHEN a hypothesis's recorded output holds no kind, because it was recorded before adapters proposed kinds
- THEN the hypothesis keeps no other kind and reads as a `promise`

### Requirement: One list holds every source, and the filters narrow it
One list SHALL hold every source's hypotheses, each naming its source, and SHALL narrow to the source or the vocabulary term the caller filters by, returning an empty list rather than an error when a filter matches nothing.

#### Scenario: two sources share one list
- WHEN two sources have hypotheses
- THEN one list holds both sources' hypotheses, each naming its source

#### Scenario: a source filter drops the other sources
- WHEN the caller filters by source
- THEN the list narrows to that source and drops the other sources' entries

#### Scenario: a term filter narrows the hypotheses
- WHEN the caller filters by vocabulary term
- THEN the hypotheses narrow to those expressing that term, and a filter that matches nothing returns an empty list, not an error

#### Scenario: a failed source survives the term filter
- WHEN a term filter runs and a source's last run looked but no hypothesis of it expresses the term, or a source's last run failed
- THEN the looking source drops and the failed source stays, because its gap is part of the answer

### Requirement: A gap is named, never left out
The list SHALL name a source that has no hypotheses, whose last run failed or whose coverage couldn't look — `found nothing`, the failed run with its cause, or `couldn't look` with its reason — and SHALL never leave a source out silently.

#### Scenario: a source with nothing is named
- WHEN a source has no hypotheses, its extraction succeeded and its coverage found the turns
- THEN the list names that source `found nothing`

#### Scenario: a failed run or a blocked look names its cause
- WHEN a source's last run failed, or its coverage couldn't look
- THEN the list names the failed run with its cause, or `couldn't look` with its reason, and the source is never left out silently

### Requirement: A scope that hides a source hides its hypotheses
The list SHALL leave out the hypotheses of a source the launch scope hides, and SHALL still succeed.

#### Scenario: a hidden source's hypotheses are absent and the list succeeds
- WHEN the launch scope hides a source
- THEN its hypotheses are absent from the list, and the list still succeeds

### Requirement: The standard vocabulary defines the commitment terms
The standard vocabulary SHALL define `std:Commitment` and the predicates `std:owes`, `std:owed_to`, `std:due` and `std:waiting_on`, each with a definition, what it is not, examples and the evidence it needs, and every hypothesis SHALL carry the terms it expresses: `std:owes` when it has a responsible party and is not a waiting-on, `std:owed_to` when it has a recipient, `std:due` when it states a date, and `std:waiting_on` when it is a waiting-on.

#### Scenario: the bundled vocabulary holds the commitment terms
- WHEN the standard vocabulary is loaded
- THEN it defines `std:Commitment` and the predicates `std:owes`, `std:owed_to`, `std:due` and `std:waiting_on`, each with a definition, what it is not, examples and the evidence it needs

#### Scenario: every hypothesis carries the terms it expresses
- WHEN a hypothesis is read
- THEN it carries `std:owes` when it has a responsible party and is not a waiting-on, `std:owed_to` when it has a recipient, `std:due` when it states a date, and `std:waiting_on` when it is a waiting-on

### Requirement: The core calls the adapter and keeps only the candidates that pass its checks
The core SHALL call the owner's configured agent tool adapter over the plugin protocol with the content, its own instructions and the fields it expects, and SHALL keep only the candidates that pass the core's own checks: an exact contiguous quote, the owner's involvement and a stated date the quote states.

#### Scenario: only the candidates the core's checks pass are kept
- WHEN the owner imports a conversation with a configured agent tool adapter
- THEN the core calls the adapter over the plugin protocol with the content, its own instructions and the fields it expects, and keeps only the candidates that pass the core's own checks: an exact contiguous quote, the owner's involvement and a stated date the quote states

#### Scenario: a response that breaks the shape fails the run
- WHEN an adapter's response breaks the expected shape, never answers, or answers with an error
- THEN the run fails and the last good results stay

### Requirement: An adapter's manifest declares its kind and names no tool
An adapter's manifest SHALL declare the kind `agent-tool-adapter` and the agent-tool capabilities it needs, never a named tool, and a manifest that names a tool or doesn't declare the kind SHALL be refused before anything runs.

#### Scenario: a manifest that names a tool or lacks the kind is refused
- WHEN an adapter's manifest is checked
- THEN a manifest that names a tool, or that doesn't declare the kind, is refused before anything runs

### Requirement: The interpretation is recorded, replayed without a model and kept out of exports
Every successful run, replay and projection rebuild SHALL record the interpretation with the spans it cites; a replay SHALL check it again with no model call, and SHALL fail and mark the results stale when the record no longer validates; a rebuild SHALL never call a model; and an export SHALL leave the recorded output and the spans out.

#### Scenario: a run records its interpretation with its spans
- WHEN a run succeeds
- THEN the interpretation is recorded with the spans it cites

#### Scenario: a replay checks the record with no model call
- WHEN recorded output is replayed
- THEN it is checked again with no model call

#### Scenario: a record that no longer validates fails stale
- WHEN a replay's record no longer validates
- THEN the replay fails and marks the results stale

#### Scenario: a rebuild calls no model
- WHEN the projection is rebuilt
- THEN no model is called

#### Scenario: an export leaves the output and the spans out
- WHEN a database is exported
- THEN the recorded output and the spans stay out

### Requirement: A party mapped to a bot excludes the candidate
The core SHALL exclude and count a candidate whose party is explicitly mapped to an account the evidence says is a bot, and SHALL keep a party that is only named.

#### Scenario: a mapped bot excludes and a named party stays
- WHEN a candidate's party is explicitly mapped to an account the evidence says is a bot
- THEN the candidate is excluded and counted; a party that is only named is not excluded

### Requirement: The owner reads a real conversation's commitments over MCP
The owner SHALL read the commitments of a real conversation from the `commitments` list over MCP, without opening the transcript by hand.

#### Scenario: the list alone carries a conversation's hypotheses
- WHEN the owner asks the MCP tool `commitments` for a conversation they imported
- THEN every hypothesis comes back with the exact quote it rests on, its turn and its speaker, so the transcript is never opened by hand
