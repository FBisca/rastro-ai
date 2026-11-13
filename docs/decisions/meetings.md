# Meetings: commitments from a transcript

Status: agreed 2026-11-13 (plan `meeting-commitments`, owner answers Q1–Q29). North star: `../concept.md` ("Commitments from conversations"). Architecture: `../adr/0011-meeting-commitment-hypotheses.md`.

## Decisions
1. **Manual, speaker-labelled local text first.** The owner imports one transcript file for one meeting, with an identifier he chooses, its capture time and its scope. No provider integration; a real meeting is exercised before anything is built around one.
2. **Both directions, unknown counterparties included.** A hypothesis is a commitment the owner made or was promised; the other party may be unknown. Promises between other people are left out.
3. **Identity is explicit.** The owner names his own speaker label; other labels are mapped one by one to existing entities. An unmapped label stays unknown and is shown by its label. A name never identifies anyone, even when a Person of that name exists.
4. **Exact quotes, stated dates.** Every hypothesis carries one exact, contiguous quote with its turn and speaker. A date stays as said ("next Friday") and is never resolved; one the quote does not state reads unknown. A bad quote fails the whole run, so a result never rests on text that is not in the transcript.
5. **A failed run keeps the last good results, visibly.** The latest run's state and cause are always shown, and results from an earlier run say so.
6. **The extractor is configured, and the transcript is data.** The owner configures the command; supplying the file is the authorization (no separate owner rule). Nothing in a transcript instructs Rastro. Conditions stay in the promise; uncertain repeats stay separate.
7. **Results are hypotheses, and say what was looked at.** Every read labels them hypotheses and reports extraction state and coverage (turns read of turns total; `found`, `empty` or `couldn't look`). Nothing says a commitment is open, done or tracked, or that the list is complete.
8. **Acceptance is a reviewed real meeting plus automated fixtures,** not a numeric threshold from one meeting.

## Out of scope
Provider integrations, a review queue, a correction UI, and any claim that a commitment is open, done or untracked.

## Parked for the next ticket
Re-extraction with stale state, replay of recorded output without a model call, retention of spans against the full transcript, and forgetting a meeting.
