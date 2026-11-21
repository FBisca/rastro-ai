# Commitments from Slack threads
Lane: Plan, big and complex: it widens the commitments pipeline and its storage beyond meetings.

Commitment hypotheses come only from imported meeting transcripts today, so promises made in Slack threads are invisible. This plan lets the owner opt a chat channel in; after each sync, the tick sends its changed threads to the bound adapter, and the hypotheses appear in the same commitments list as meetings, with speakers identified by account rather than by name. Meetings move into a shared conversation store and behave exactly as before. Four tickets, after the Slack connector, Slack identity and the in-flight promise-identity plan.

## Your calls
None.

## What changes

### The owner
- **New:** a channel's threads go to the adapter only when the owner added it with `--extract-commitments` (S3, S4).
- **New:** extracted threads show in the commitments list as sources of kind `chat`, named by channel and thread (S4), and a reader limited to other scopes doesn't see them (S5).
- **New:** a thread that gains a reply or an edited reply is extracted again and its hypotheses replaced (S6, S8); nothing unchanged is sent again (S7).
- **New:** a thread the owner isn't in, or one too large, is skipped and counted in coverage, not a failed run (S15, S16).

### The model's input
- **New:** one turn per message, so a multi-line message can't invent a speaker (S9).
- **New:** two authors with the same name get labels that differ by id (S10), and mentions read as names (S11).

### Who is responsible
- **New:** the token owner's promises are `me`'s (S12); another author's ask names that author's account and `me` as recipient (S13).
- **New:** a candidate whose party is a bot account is excluded and counted (S14).

### Meetings
- **Changed:** meetings live in the shared conversation store; their hypotheses, staleness, coverage and replay are unchanged (S1, S2).

## Walkthrough
1. The owner adds `#eng` with `--extract-commitments`; a sync reads a thread where Ana asks the owner for the RBAC doc and the owner replies "I'll send it Friday" (S4).
2. The tick builds two turns, labelled `Ana Duarte` and the owner's name, and sends them to the bound adapter (S9, S4).
3. The list shows a `chat` source for the thread with an ask by Ana's account to `me` and a promise by `me` (S13, S12).
4. Ana replies "thanks"; the next tick extracts the thread again and replaces its hypotheses (S6).

What can go wrong:
- `#eng` wasn't opted in: nothing goes to the adapter (S3).
- A thread between two colleagues: skipped and counted, owner absent (S15).
- The deploy bot "promises" a release: excluded and counted (S14).
- A 6 MiB incident thread: skipped as too large (S16).

## Out of scope
- Forgetting one chat thread and its Slack evidence: slice 14.
- Slack DMs and group DMs: never read (`native-slack`).
- Commitments from GitHub or Linear comments: parking lot.
- Chunking threads over the transcript cap.

## Risks
- **Text leaving the machine:** thread text goes to the bound adapter; only opted-in channels send anything (S3), mirroring meetings where supplying the file is the authorization.
- **Migration:** moving meetings into the conversation store could change existing results; S1 and S2 pin them on a pre-migration database.
- **Conflict with promise-identity:** both touch the commitments code and meeting storage; this plan builds after it merges (D1).
- **Re-extraction cost:** busy threads re-run the adapter on every change; only changed threads run (S7).

---

# Reference

## Build order
- T1 meetings move into the conversation store, with the ADR 0011 amendment, the Conversation glossary term and the meetings §3 chat amendment (regular · complex).
- T2 the opt-in, the tick extracting new threads, and `chat` sources in the list and schema (regular · complex; after T1).
- T3 turns from message evidence, account identity, bot exclusion and skip rules (regular · complex; after T2).
- T4 change detection and re-extraction (regular · complex; after T3).

Across plans: after `slack-identity` T1 and the `promise-identity` plan.

## Decision log
- **D1 Built after the Slack connector and after the promise-identity plan merges.** Rejected: building in parallel. Why: it needs Slack evidence, and promise-identity changes the same commitments code and meeting storage. Source: user, round 1 Q1; gather (`crates/rastro-core/src/commitments/mod.rs`, `crates/rastro-storage-sqlite/src/meeting.rs`).
- **D2 One conversation is one thread.** The starting message plus its replies; a message with no replies is not extracted. Rejected: a channel window. Source: user, round 2 Q4.
- **D3 Per-channel opt-in authorizes sending, and the tick extracts after sync.** `--extract-commitments` at source add. Rejected: extracting every synced channel; a manual command per thread. Why: mirrors meetings §6, where supplying the file is the authorization. Source: user, round 2 Q5; `docs/decisions/meetings.md` §6.
- **D4 Speakers come from evidence, not from text.** Turns are built from message evidence; display-name labels with the id appended on a shared name; the owner is the token owner; other authors map by account id. Rejected: serializing to `Label: text`, which splits multi-line messages into fake speakers (`crates/rastro-core/src/meeting.rs:627-638`); `--owner-speaker` per thread. Source: user, round 2 Q6.
- **D5 Mentions are resolved in the turn text.** Quotes are checked against that text and the cited turns are recorded for replay; raw evidence stays unchanged. Rejected: raw mention syntax, which breaks the owner rules that match labels. Source: user, round 2 Q7; `crates/rastro-core/src/meeting_rules_v1.toml`.
- **D6 Skips are counted, not failures.** Bot parties excluded by the existing rule; owner-absent and oversize threads skipped and counted in coverage. Rejected: failing the run with `owner_not_in_transcript`. Source: user, round 2 Q8.
- **D7 Changed threads are re-extracted and replace their hypotheses.** Rejected: extract once. Source: user, round 2 Q9.
- **D8 Kind `chat`, one conversation store, Conversation in the glossary.** Rejected: kind `slack`; parallel chat tables. Why: a connector-neutral kind and one store keep reads, replay and forget in one place; recorded as an ADR 0011 amendment. Source: user, round 2 Q10.
- **D9 A connector-neutral chat evidence convention.** `message` evidence with `/text`, `/user`, `/thread_ts` at `<ns>:message/...`, authors at `<ns>:user/<id>`, the owner at `<ns>:viewer`. Rejected: the core reading Slack's locations directly. Why: the core should hold no connector-specific code, the split the owner asked for; the viewer convention already works this way. Source: user, approval round; `docs/decisions/plugins.md` §25.

## Code touched
- Storage: `crates/rastro-storage-sqlite/src/meeting.rs`, `src/commitments.rs`, `migrations/` (8 meeting tables), `tests/migrations.rs`.
- Core: `crates/rastro-core/src/meeting.rs` (import split from run and record), `src/commitments/` (`SourceKind`, chat assembly), `src/host/task.rs` (generic runner, unchanged), `src/tick.rs` (no extraction hook today).
- Operations and CLI: `crates/rastro-operations/` (source add flag, commitments output), `crates/rastro-cli/src/main.rs` (kind rendering); `schema/operations-v2.schema.json` (`SourceKind` gains `chat`), recorded in `docs/decisions/surfaces.md`.
- Tests: `crates/rastro-cli/tests/meeting.rs` (fake adapter script), new `crates/rastro-cli/tests/chat_commitments.rs`; chat evidence from `crates/rastro-test-connector/`.
- Docs: `docs/adr/0011-meeting-commitment-hypotheses.md`, `docs/GLOSSARY.md`, `docs/decisions/meetings.md` §3.
- Spec: `docs/specs/commitments.md` (added requirements).

## Interfaces

| Item | Value |
|---|---|
| Opt-in | `rastro source add <name> --connector <id> --extract-commitments`, stored on the source |
| Chat evidence | kind `message` at `<ns>:message/<channel>/<ts>` with `/text` (untrusted), `/user`, `/thread_ts`; authors at `<ns>:user/<id>`; owner at `<ns>:viewer` |
| Thread identity | channel source + thread start `ts` |
| List entry | `{kind: "chat", name: "<source>/<thread ts>", scope: <source scope>, ...}`, same shape as meetings |
| Skip reasons | owner absent; over the transcript cap (4 MiB) |
