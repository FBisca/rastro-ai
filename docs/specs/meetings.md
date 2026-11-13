# Meetings

Commitment hypotheses about the owner, proposed by a configured extractor from one speaker-labelled meeting transcript and checked against it. A hypothesis is not a confirmed commitment, and nothing here says whether one is open, done or tracked. Product meaning: `../decisions/meetings.md`. Architecture: `../adr/0011-meeting-commitment-hypotheses.md`.

## Reviewed meetings

One real meeting is reviewed against an expected list before the slice is accepted. The record keeps what the review found, not a threshold.

### RBAC Q4 kickoff — 2026-10-23

Owner speaker `Felippe Bisca`; a 19-minute Teams transcript, 48 turns, four speakers. The import succeeded with coverage `found` (48 of 48 turns), no excluded candidates and seven hypotheses; the CLI and the MCP tool returned the same result.

Against the owner's expected list, approved before the import:

- Found: explain why the name doesn't exist after the meeting (turn 1); add the owner field to existence (turn 18); "I will show you in a bit" twice (turns 46 and 48, kept separate); "I'll show you a very brief demo" (turn 48). Nothing was proposed from Nathan Beale, Giuseppe Pirrelli or Carlo Elefanti, as expected.
- Omitted: discuss the book of clients (turn 48). The extractor proposed nothing for "This is something that I have to discuss with you, but all the clients that you've been managing."
- Extra: "I'll try to give an overview … and then I'll explain why this name doesn't exist" (turn 1) and "I will go through the things that I've been building the last quarter and this quarter" (turn 5). Both are promises in the text; the expected list was narrower.

All seven quotes were exact and contiguous pieces of the named turn. Two promises name more than their quote does: "put in here" became a document, and "show you in a bit" became the business relationship. Recipients read unknown in every hypothesis: the promises address the room, and the extractor proposed no recipient label. No counterparty promised the owner anything in this transcript, so it does not exercise the inbound direction; the automated scenarios do.

## Requirements: meeting-commitments

- **R1** WHEN a user imports a speaker-labelled transcript for one meeting with an identifier, capture time, explicit owner mapping and configured extractor, THEN CLI and agent-facing reads show owner-involved commitment hypotheses in both directions with responsible person, recipient, promise, stated date and exact supporting quote, marking unclear fields unknown. (from meeting-commitments/S1)
- **R2** WHEN the owner is unmapped, the extractor is unconfigured, or a source read fails, THEN extraction does not claim success and the user sees the cause and incomplete coverage. (from meeting-commitments/S2)
- **R3** WHEN a proposed candidate has no owner involvement supported by the transcript, or has an unverifiable or noncontiguous quote, THEN an unsupported owner relationship is excluded and an invalid quote fails the entire run without replacing last-good results. (from meeting-commitments/S3)
- **R4** WHEN a transcript contains relative dates, conditional promises, ambiguous speakers, repeated promises, or embedded instructions, THEN dates remain as stated, conditions remain in promises, unresolved people remain unknown, uncertain repeats remain separate, and transcript instructions do not control extraction. (from meeting-commitments/S4)
- **R5** WHEN a meeting is re-imported with changed text and extraction fails or succeeds, THEN last-good results remain marked stale on failure and are replaced by the successful new execution on success, without duplicating the meeting. (from meeting-commitments/S5)
- **R6** WHEN recorded output is replayed or the meeting is forgotten, THEN replay requires no model call and forgetting removes retained spans and recorded output for the meeting. (from meeting-commitments/S6)
- **R7** WHEN a meeting is imported without full-transcript retention opt-in, THEN supporting spans and recorded output are retained for replay, while the full transcript is not retained, and exports omit recorded model output. (from meeting-commitments/S7)
- **R8** WHEN results are read for a meeting, THEN the surface labels them hypotheses and reports extraction and coverage state without asserting completion or tracking status. (from meeting-commitments/S8)
- **R9** WHEN a speaker label is explicitly mapped or left unmapped, THEN the mapped existing identity is used and an unmapped person remains unknown without name-based identity inference. (from meeting-commitments/S9)

## Coverage: meeting-commitments

| Requirement | Scenario | Test nodes |
|---|---|---|
| R1 | S1 | `crates/rastro-cli/tests/meeting.rs::S1_cli_and_agent_reads_show_owner_involved_hypotheses_in_both_directions`<br>human: Run the documented CLI and agent-facing flow on a reviewed real meeting; compare every result and omission against the expected list and inspect both surfaces. |
| R2 | S2 | `crates/rastro-cli/tests/meeting.rs::S2_failures_name_their_cause_and_report_incomplete_coverage`<br>human: Exercise each failure through the public interface and confirm the reported cause and incomplete coverage. |
| R3 | S3 | `crates/rastro-cli/tests/meeting.rs::S3_unsupported_candidates_are_excluded_and_an_invalid_quote_fails_the_run`<br>human: Exercise each case through the public interface and inspect the failure and retained results. |
| R4 | S4 | `crates/rastro-cli/tests/meeting.rs::S4_stated_dates_conditions_unknown_people_repeats_and_instructions_stay_as_given`<br>human: Inspect public results for each case against the source and verify the extractor treats instructions as data. |
| R5 | S5 | `crates/rastro-cli/tests/meeting.rs::S5_reimport_with_changed_text_marks_last_good_stale_on_failure_and_replaces_on_success`<br>human: Re-import changed text, force a failure and then a success, and inspect identity, staleness and results. |
| R6 | S6 | `crates/rastro-cli/tests/meeting.rs::S6_replay_needs_no_model_call_and_forgetting_removes_what_was_retained`<br>`crates/rastro-cli/tests/meeting.rs::S6_replay_without_recorded_output_is_an_error_and_records_nothing`<br>`crates/rastro-cli/tests/meeting.rs::S6_a_replay_that_no_longer_validates_fails_and_marks_the_results_stale`<br>human: Replay with the extractor unavailable, then forget the meeting and inspect retained data and subsequent reads. |
| R7 | S7 | `crates/rastro-cli/tests/meeting.rs::S7_default_import_retains_spans_and_output_but_not_the_transcript`<br>human: Inspect persisted data and an export after default import, then verify opt-in retains the transcript. |
| R8 | S8 | `crates/rastro-cli/tests/meeting.rs::S8_reads_are_labelled_hypotheses_with_extraction_and_coverage_state`<br>human: Inspect CLI and agent-facing results for successful, stale, failed and incomplete cases. |
| R9 | S9 | `crates/rastro-cli/tests/meeting.rs::S9_mapped_speakers_use_the_existing_identity_and_unmapped_ones_stay_unknown`<br>human: Compare results for mapped, unmapped and same-name speakers against their explicit mappings. |
