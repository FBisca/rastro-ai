---
status: accepted
date: 2026-11-13
---

# Meeting commitment hypotheses: a one-shot extractor command, kept outside the claims projection

A user imports one speaker-labelled transcript for one meeting. A configured extractor proposes commitments that involve the owner; the core checks each proposal against the transcript and keeps the survivors as **commitment hypotheses**, each with its exact quote. Product meaning: `../decisions/meetings.md`.

## Shape
1. **Operations (catalog, ADR 0007).** `meeting.import` (CLI only, writes) and `meeting.commitments` (CLI and MCP, read-only, on the CLI's read path). `meeting.import` returns the same result a later `meeting.commitments` read returns, and the CLI exits non-zero when its run failed. A run that fails is a result, not an error: it is recorded with its cause (`source_unreadable`, `transcript_invalid`, `owner_unmapped`, `owner_not_in_transcript`, `mapping_invalid`, `extractor_unconfigured`, `extractor_failed`, `extractor_output_invalid`, `invalid_quote`). Only a request that cannot name a meeting (empty identifier or scope, a speaker mapped twice, a bad `--speaker` form) is an error and records nothing.
2. **Transcript format.** Plain text, one turn per `Label: text` line (the colon followed by white space or the end of the line). Any other non-blank line continues the turn before it. At most 4 MiB of UTF-8.
3. **Extractor adapter.** `config.toml` holds `[meeting_extractor]` with `command`, optional `args` and `timeout_seconds` (default 120). The core runs it once per import, writes one JSON request on stdin and reads one JSON response from stdout (at most 4 MiB). The request has the protocol name, the core's `instructions`, the meeting, the owner's speaker label and the `turns`; the transcript text appears only in `turns[].text`. The response is `{"candidates": [{turn, responsible, recipient, promise, stated_date, quote}]}`, unknown fields denied. The configured command is the owner's authorization to send transcripts to it; it is not a plugin and holds no grants (ADR 0004), so it is not offered to anything but this operation.
4. **What the core checks, itself.**
   - The quote is a non-empty, exact, contiguous piece of the named turn's text. Any candidate that fails (even one the owner is not in) fails the whole run.
   - The owner's label is the responsible person or the recipient, and the quoted turn is the owner's or the other named party's. Otherwise the candidate is left out and counted (`excluded`), and nothing of it is stored.
   - A stated date is kept only if it is a piece of the quote; otherwise it reads unknown.
   - People: the owner is the Person `me`; other labels resolve only through explicit `--speaker label=entity` mappings to existing entities. A label that is not mapped, or that the extractor names without it being a speaker, stays `unknown` and is shown by its label. Names never resolve to identities.
   - Candidates are never merged or reordered; repeats stay separate hypotheses.
5. **Storage.** Migration `202611131000_meeting_commitments`: `meeting` (identifier, scope, capture time), `meeting_run` (state, cause, extractor, turns read and total, excluded count) and `meeting_hypothesis` (per run: both people as the label and the resolved entity, promise, stated date, turn, speaker, quote and its byte span in the turn). A successful run replaces the hypotheses of earlier runs in the same transaction; a failed run adds only its run row. A read shows the latest run's state and the hypotheses of the latest successful run, saying so (`results_from`) when they differ. The transcript is not stored.
6. **Scope.** A meeting belongs to the scope given at import. A reader limited to other scopes (`rastro mcp --scope`) gets the same error for it as for an unknown meeting. Importing an existing identifier in another scope is refused.

## Considered options
- **Route hypotheses through the claims projection (ADR 0009).** Rejected: the projection takes deterministic extractors over structured, insert-only evidence (ADR 0008), while this input is untrusted, the output is not reproducible, and the supporting text has to be forgettable. A later slice can promote an accepted hypothesis into a claim with its own acceptance rule (ADR 0008's note on slice 5).
- **A manifest-declared extractor plugin.** Heavier than the need and grants nothing useful here; the configured command is explicit and narrow. Revisit with the agent tool adapter.
- **Core-side name matching to find people.** Rejected: it would infer identity from names (`../decisions/knowledge.md` §54).

## Consequences
- Recorded output, replay without a model call, stale results, retention and forgetting build on `meeting_run` and `meeting_hypothesis` (the follow-up ticket); this ADR is amended there.
- The owner's mapping is per import, not remembered.
