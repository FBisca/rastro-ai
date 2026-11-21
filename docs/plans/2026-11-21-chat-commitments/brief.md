# Commitments from Slack threads
Lane: Plan, big and complex: it widens the commitments pipeline and its storage beyond meetings.

Commitment hypotheses come only from imported meeting transcripts today, so promises made in Slack threads are invisible. This plan lets the owner opt a chat channel in; after each sync, the tick asks the source's extractor for the turns of each changed thread, checks them, and sends them to the bound adapter. The hypotheses appear in the same commitments list as meetings, with speakers identified by account rather than by name. A conversation is a contract: the core defines turns, a source plugin makes them, and the core keeps the gates, the queue and accept. Meetings move into a shared conversation store and behave exactly as before. Five tickets, after Slack identity.

Re-planned 2026-11-22 from the owner note `owner-note-2026-11-21.md`: turn-making moved from the core to the Slack extractor (D10–D14).

## Your calls
None.

## What changes

### The owner
- **New:** a channel's threads go to the adapter only when the owner added it with `--extract-commitments` (S3, S4), and only if its extractor can make turns (S18).
- **New:** extracted threads show in the commitments list as sources of kind `chat`, named by channel and thread (S4), and a reader limited to other scopes doesn't see them (S5).
- **New:** a thread that gains a reply or an edited reply is extracted again and its hypotheses replaced (S6, S8); nothing unchanged is sent again (S7).
- **New:** a thread the owner isn't in, or one too large, is skipped and counted in coverage, not a failed run (S15, S16).

### The model's input
- **New:** the Slack extractor makes one turn per message, so a multi-line message can't invent a speaker (S9).
- **New:** two authors with the same name get labels that differ by id (S10), and mentions read as names (S11).
- **New:** a turn that cites a message outside its thread is refused, and the thread is skipped and counted (S17).

### Who is responsible
- **New:** turns by an account the owner's Person holds are `me`'s (S12); another author's ask names that author's account and `me` as recipient (S13).
- **New:** a candidate whose party is a bot turn is excluded and counted (S14).

### Meetings
- **Changed:** meetings live in the shared conversation store, and their turns use the same contract; their hypotheses, staleness, coverage and replay are unchanged (S1, S2).

## Walkthrough
1. The owner adds `#eng` with `--extract-commitments`; a sync reads a thread where Ana asks the owner for the RBAC doc and the owner replies "I'll send it Friday" (S4).
2. The tick asks the Slack extractor for the thread's turns. It returns two turns, `Ana Duarte` and the owner's account, each citing its message (S9).
3. The core checks the turns, finds the owner's account, and sends them to the bound adapter (S4).
4. The list shows a `chat` source for the thread with an ask by Ana's account to `me` and a promise by `me` (S13, S12).
5. Ana replies "thanks"; the next tick extracts the thread again and replaces its hypotheses (S6).

What can go wrong:
- `#eng` wasn't opted in: nothing goes to the adapter (S3).
- The source's extractor can't make turns: the opt-in fails (S18).
- A thread between two colleagues: skipped and counted, owner absent (S15).
- The deploy bot "promises" a release: excluded and counted (S14).
- A 6 MiB incident thread: skipped as too large (S16).

## Out of scope
- A transcript connector for meetings: `meeting.import` stays a core operation (D12).
- A plugin-declared involvement rule: the core runs one rule (D13).
- Forgetting one chat thread and its Slack evidence: slice 14.
- Slack DMs and group DMs: never read (`native-slack`).
- Commitments from GitHub or Linear comments: parking lot.
- Chunking threads over the transcript cap.

## Risks
- **Text leaving the machine:** thread text goes to the bound adapter; only opted-in channels send anything (S3), mirroring meetings where supplying the file is the authorization.
- **Trusting the extractor:** the extractor is deterministic code, but its turns decide who spoke. The core refuses turns outside the thread (S17) and decides involvement and bots itself (S12, S14, S15).
- **Migration:** moving meetings into the conversation store could change existing results; S1 and S2 pin them on a pre-migration database.
- **Re-extraction cost:** busy threads re-run the adapter on every change; only changed threads run (S7).

---

# Reference

## Build order
- T1 the conversation contract (SDK request and turn type), meetings in the conversation store with their turns in the contract, the ADR 0011 amendment, the Conversation glossary term and the meetings §3 chat amendment (regular · complex).
- T2 the Slack extractor answers `conversation.turns` (regular · complex; after T1).
- T3 the opt-in, the tick asking the extractor for new threads' turns, and `chat` sources in the list and schema (regular · complex; after T2).
- T4 the core's checks: refused turns, owner by held account, bots, skips (regular · complex; after T3).
- T5 change detection and re-extraction (regular · complex; after T4).

Across plans: after the `slack-identity` plan (it creates `crates/rastro-extractor-slack`).

## Decision log
- **D1 Built after the Slack connector and Slack identity.** Rejected: building in parallel. Why: it needs Slack evidence and the Slack extractor. Was also after promise-identity; the owner deleted that plan on 2026-11-22, so the dependency is dropped. Source: user, round 1 Q1; owner, 2026-11-22.
- **D2 One conversation is one thread.** The starting message plus its replies; a message with no replies is not extracted. Rejected: a channel window. Source: user, round 2 Q4.
- **D3 Per-channel opt-in authorizes sending, and the tick extracts after sync.** `--extract-commitments` at source add. Rejected: extracting every synced channel; a manual command per thread. Why: mirrors meetings §6, where supplying the file is the authorization. Source: user, round 2 Q5; `docs/decisions/meetings.md` §6.
- **D4 Speakers come from evidence, not from text (amended by D10).** One turn per message; display-name labels with the id appended on a shared name; other authors by account id. Rejected: serializing to `Label: text`, which splits multi-line messages into fake speakers (`crates/rastro-core/src/meeting.rs`); `--owner-speaker` per thread. Source: user, round 2 Q6.
- **D5 Mentions are resolved in the turn text (made by the extractor since D10).** Quotes are checked against that text and the cited turns are recorded for replay; raw evidence stays unchanged. Rejected: raw mention syntax, which breaks the owner rules that match labels. Source: user, round 2 Q7; `crates/rastro-core/src/meeting_rules_v1.toml`.
- **D6 Skips are counted, not failures.** Bot parties excluded; owner-absent and oversize threads skipped and counted in coverage. Rejected: failing the run with `owner_not_in_transcript`. Source: user, round 2 Q8.
- **D7 Changed threads are re-extracted and replace their hypotheses.** Rejected: extract once. Source: user, round 2 Q9.
- **D8 Kind `chat`, one conversation store, Conversation in the glossary.** Rejected: kind `slack`; parallel chat tables. Why: a connector-neutral kind and one store keep reads, replay and forget in one place; recorded as an ADR 0011 amendment. Source: user, round 2 Q10.
- **D9 Superseded by D11.** Was: a connector-neutral chat evidence convention that the core reads.
- **D10 The Slack extractor makes chat turns.** Rejected: the Slack connector, which only captures; the core. Why: the extractor already reads Slack evidence and resolves authors to `slack:Account` (`slack-identity` T1); turn-making belongs to the source's plugin like keys (`docs/decisions/plugins.md` §54). Source: owner note 2026-11-21; user, re-plan round Q1.
- **D11 The contract is `conversation.turns`.** Core to extractor: a thread's evidence in, turns out (speaker account or label, kind `person` or `bot`, text with mentions resolved, time, reply-to, cited evidence, mentioned accounts). The core refuses a turn citing outside its thread. Rejected: D9's convention, connector-shaped code in the core. Source: user, re-plan round Q2.
- **D12 `meeting.import` stays a core operation.** Its turns enter the same contract inside the core. Rejected: a transcript connector now. Why: short scope; meetings must not change (S1, S2). Source: user, re-plan round Q3.
- **D13 The core runs one involvement rule.** The owner is in a thread when a turn's speaker or a mentioned account is held by the owner's Person; meetings keep their v1 label rules (`docs/decisions/plugins.md` §49). Rejected: a plugin-declared rule, which needs a rule format. Source: user, re-plan round Q4.
- **D14 Bots are a turn kind.** The extractor marks `slack:Bot` speakers as kind `bot`; the core excludes a candidate whose party is a bot turn. Rejected: the core knowing Slack's bot type. Source: user, re-plan round Q5.

## Code touched
- SDK: `crates/rastro-plugin-sdk/src/` (`conversation.turns` request, turn type).
- Storage: `crates/rastro-storage-sqlite/src/meeting.rs`, `src/commitments.rs`, `migrations/` (8 meeting tables), `tests/migrations.rs`.
- Core: `crates/rastro-core/src/meeting.rs` (import split from run and record), `src/commitments/` (`SourceKind`, chat assembly, contract checks), `src/host/extract.rs` (calling the extractor), `src/host/task.rs` (generic runner, unchanged), `src/tick.rs` (no extraction hook today).
- Slack extractor: `crates/rastro-extractor-slack/` (created by `slack-identity`).
- Operations and CLI: `crates/rastro-operations/` (source add flag, commitments output), `crates/rastro-cli/src/main.rs` (kind rendering); `schema/operations-v2.schema.json` (`SourceKind` gains `chat`), recorded in `docs/decisions/surfaces.md`.
- Tests: `crates/rastro-cli/tests/meeting.rs` (fake adapter script), new `crates/rastro-cli/tests/chat_commitments.rs`; chat evidence from `crates/rastro-test-connector/`.
- Docs: `docs/adr/0011-meeting-commitment-hypotheses.md`, `docs/GLOSSARY.md`, `docs/decisions/meetings.md` §3.
- Spec: `docs/specs/commitments.md` (added requirements).

## Interfaces

| Item | Value |
|---|---|
| Opt-in | `rastro source add <name> --connector <id> --extract-commitments`, stored on the source; refused when the bound extractor lacks `conversation.turns` |
| `conversation.turns` request | the thread's identity and its message evidence |
| `conversation.turns` turn | `speaker` (account entity or label), `kind` (`person` or `bot`), `text` (mentions resolved; untrusted), `time`, `reply_to`, `cites` (evidence location), `mentions` (accounts) |
| Thread identity | channel source + thread start `ts` |
| List entry | `{kind: "chat", name: "<source>/<thread ts>", scope: <source scope>, ...}`, same shape as meetings |
| Skip reasons | owner absent; over the transcript cap (4 MiB); a turn refused |
