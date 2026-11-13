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
