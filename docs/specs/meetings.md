# Meetings

Commitment hypotheses about the owner, proposed by a configured extractor from one speaker-labelled meeting transcript and checked against it. A hypothesis is not a confirmed commitment, and nothing here says whether one is open, done or tracked. Each hypothesis carries its kind (`ask`, `promise` or `waiting_on`). The cross-source read of every meeting's hypotheses is the `commitments` list (`commitments.md`); `meeting.commitments` stays until the next schema version. Product meaning: `../decisions/meetings.md`. Architecture: `../adr/0011-meeting-commitment-hypotheses.md`.

## Reviewed meetings

One real meeting is reviewed against an expected list before the slice is accepted. The record keeps what the review found, not a threshold.

### RBAC Q4 kickoff — 2026-10-23

Owner speaker `Felippe Bisca`; a 19-minute Teams transcript, 48 turns, four speakers. The import succeeded with coverage `found` (48 of 48 turns), no excluded candidates and seven hypotheses; the CLI and the MCP tool returned the same result.

Against the owner's expected list, approved before the import:

- Found: explain why the name doesn't exist after the meeting (turn 1); add the owner field to existence (turn 18); "I will show you in a bit" twice (turns 46 and 48, kept separate); "I'll show you a very brief demo" (turn 48). Nothing was proposed from Nathan Beale, Giuseppe Pirrelli or Carlo Elefanti, as expected.
- Omitted: discuss the book of clients (turn 48). The extractor proposed nothing for "This is something that I have to discuss with you, but all the clients that you've been managing."
- Extra: "I'll try to give an overview … and then I'll explain why this name doesn't exist" (turn 1) and "I will go through the things that I've been building the last quarter and this quarter" (turn 5). Both are promises in the text; the expected list was narrower.

All seven quotes were exact and contiguous pieces of the named turn. Two promises name more than their quote does: "put in here" became a document, and "show you in a bit" became the business relationship. Recipients read unknown in every hypothesis: the promises address the room, and the extractor proposed no recipient label. No counterparty promised the owner anything in this transcript, so it does not exercise the inbound direction; the automated scenarios do.

## Requirements

### Requirement: CLI and agent reads show owner-involved hypotheses in both directions
Importing a speaker-labelled transcript for one meeting with an identifier, capture time, explicit owner mapping and a configured extractor SHALL make the CLI and the agent-facing reads show owner-involved commitment hypotheses in both directions, with the responsible person, the recipient, the promise, the stated date and the exact supporting quote, marking unclear fields unknown.

#### Scenario: both directions read alike on the CLI and over MCP
- WHEN a user imports a speaker-labelled transcript for one meeting with an identifier, capture time, explicit owner mapping and a configured extractor
- THEN CLI and agent-facing reads show owner-involved commitment hypotheses in both directions with responsible person, recipient, promise, stated date and exact supporting quote, marking unclear fields unknown

### Requirement: Extraction failures name their cause and report incomplete coverage
An import SHALL not claim success when the owner is unmapped, the extractor is unconfigured or a source read fails, and SHALL show the cause and incomplete coverage.

#### Scenario: each failure says why and how little was read
- WHEN the owner is unmapped, the extractor is unconfigured, or a source read fails
- THEN extraction does not claim success and the user sees the cause and incomplete coverage

### Requirement: An unsupported candidate is excluded and an invalid quote fails the run
A proposed candidate without owner involvement supported by the transcript SHALL be excluded, and a candidate whose quote is unverifiable or noncontiguous SHALL earn capped retry turns in the same judgment session before failing the entire run without replacing last-good results.

#### Scenario: an unsupported candidate is excluded, and an invalid quote fails the run
- WHEN a proposed candidate has no owner involvement supported by the transcript, or has an unverifiable or noncontiguous quote
- THEN an unsupported owner relationship is excluded, and an invalid quote is sent back as the next turn of the same session while turns remain under the cap, and once the cap is exhausted the run fails without replacing last-good results

#### Scenario: a corrected quote on a retry turn succeeds
- WHEN a retry turn of the same session answers with a quote that is an exact, contiguous piece of its turn
- THEN the run succeeds and only the final validated output becomes evidence

### Requirement: Stated dates, conditions, unknown people, repeats and instructions stay as given
Extraction SHALL leave relative dates as stated, keep conditions in their promises, leave unresolved people unknown and uncertain repeats separate, and SHALL treat transcript instructions as data.

#### Scenario: an awkward transcript keeps its dates, conditions, people and repeats
- WHEN a transcript contains relative dates, conditional promises, ambiguous speakers, repeated promises, or embedded instructions
- THEN dates remain as stated, conditions remain in promises, unresolved people remain unknown, uncertain repeats remain separate, and transcript instructions do not control extraction

### Requirement: A re-import marks last-good results stale or replaces them, without duplicating the meeting
A meeting re-imported with changed text SHALL keep its last-good results marked stale when extraction fails, SHALL replace them with the successful new execution when it succeeds, and SHALL never duplicate the meeting.

#### Scenario: a failed re-import keeps the last good results stale
- WHEN a meeting is re-imported with changed text and extraction fails
- THEN last-good results remain marked stale and the meeting is not duplicated

#### Scenario: a successful re-import replaces them
- WHEN a meeting is re-imported with changed text and extraction succeeds
- THEN the last-good results are replaced by the successful new execution, without duplicating the meeting

### Requirement: Replay needs no model call and forgetting removes what was retained
A replay of recorded output SHALL need no model call, and forgetting a meeting SHALL remove the retained spans and recorded output for it.

#### Scenario: replay and forget
- WHEN recorded output is replayed, or the meeting is forgotten
- THEN replay requires no model call and forgetting removes retained spans and recorded output for the meeting

### Requirement: Default import retains spans and output but not the transcript
An import without full-transcript retention opt-in SHALL retain the supporting spans and the recorded output for replay, SHALL not retain the full transcript, and an export SHALL omit recorded model output.

#### Scenario: a default import keeps what replay needs and not the transcript
- WHEN a meeting is imported without full-transcript retention opt-in
- THEN supporting spans and recorded output are retained for replay, while the full transcript is not retained, and exports omit recorded model output

### Requirement: Reads are labelled hypotheses with extraction and coverage state
A read SHALL label the results as hypotheses and SHALL report the extraction and coverage state without asserting completion or tracking status.

#### Scenario: every read says hypothesis with its extraction and coverage
- WHEN results are read for a meeting
- THEN the surface labels them hypotheses and reports extraction and coverage state without asserting completion or tracking status

### Requirement: Mapped speakers use the existing identity and unmapped ones stay unknown
A mapped speaker label SHALL resolve to its existing identity, an unmapped person SHALL stay unknown, and no identity SHALL be inferred from a name.

#### Scenario: the mappings decide the identity
- WHEN a speaker label is explicitly mapped or left unmapped
- THEN the mapped existing identity is used and an unmapped person remains unknown without name-based identity inference
