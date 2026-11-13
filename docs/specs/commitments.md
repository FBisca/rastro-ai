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

- **R1** WHEN the owner asks for commitments and a source has hypotheses, THEN the list shows every hypothesis with its source (kind, name, scope), its kind, the promise text, the stated date or null, the responsible and the recipient (the label, and the entity only when the owner mapped it explicitly, else null), the exact quote with its turn, speaker and byte range, the status `hypothesis` and the standard vocabulary terms it expresses; and for each source its extraction state, its coverage state and whether its results are stale. (from commitments/S1)
- **R2** WHEN two sources have hypotheses, or the caller filters by source or by vocabulary term, THEN one list holds both sources' hypotheses, each naming its source; a source filter narrows the list to that source and drops the other sources' entries; a term filter narrows the hypotheses to those expressing that term; a filter that matches nothing returns an empty list, not an error. A term filter drops a source whose last run looked and has no hypothesis expressing the term, and keeps a source whose last run failed (its gap is part of the answer). (from commitments/S2)
- **R3** WHEN a source has no hypotheses, or its last run failed, or its coverage couldn't look, THEN the list names that source: `found nothing` (no hypotheses, extraction `succeeded`, coverage `found`), the failed run with its cause, or `couldn't look` with its reason; a source is never left out silently. (from commitments/S3)
- **R4** WHEN the launch scope hides a source, THEN its hypotheses are absent from the list, and the list still succeeds. (from commitments/S4)

## Coverage: commitments

| Requirement | Scenario | Test nodes |
|---|---|---|
| R1 | S1 | `crates/rastro-cli/tests/commitments_oracle.rs::S1_the_list_shows_every_hypothesis_with_its_source_and_state`<br>`crates/rastro-cli/tests/meeting.rs::S1_a_commitments_hypothesis_keeps_its_kind_and_defaults_to_a_promise` |
| R2 | S2 | `crates/rastro-cli/tests/commitments_oracle.rs::S2_the_list_holds_every_source_and_the_filters_narrow_it`<br>`crates/rastro-cli/tests/meeting.rs::S2_the_commitments_list_holds_every_source_and_the_filters_narrow_it` |
| R3 | S3 | `crates/rastro-cli/tests/commitments_oracle.rs::S3_a_source_with_nothing_and_a_failed_run_are_both_named`<br>`crates/rastro-cli/tests/meeting.rs::S3_a_commitments_term_filter_keeps_a_source_whose_run_failed` |
| R4 | S4 | `crates/rastro-cli/tests/commitments_oracle.rs::S4_a_scope_that_hides_a_source_hides_its_hypotheses`<br>`crates/rastro-cli/tests/meeting.rs::S4_the_commitments_list_shows_each_scope_only_to_a_server_that_holds_it` |
