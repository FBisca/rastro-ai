# Read Slack channels natively
Lane: Plan, big and complex: a new connector crate owns a credential and ingests private workspace text.

Slack is where much of the owner's day-to-day happens, and Rastro can't see it today. This plan adds a Slack connector that ships with Rastro and reads the channels the owner approves, one source per channel, using a read-only token from the owner's own Slack app. Messages and thread replies become cited evidence with honest coverage; Slack Connect channels, DMs and files are never read. Three tickets in this repository, no other people.

## Your calls
None.

## What changes

### The owner setting it up
- **New:** the connector ships a Slack app manifest template that asks only for the five read scopes, with no bot and token rotation off (S23).
- **New:** `rastro check` accepts a Slack token only when it holds exactly the read scopes, and names any extra or missing scope (S1, S2, S3).
- **New:** the token comes from `SLACK_USER_TOKEN`, else the keychain item `rastro-slack` (S4); with neither, the sync reads "couldn't look: auth" (S5).
- **New:** a Slack source names one channel by its id; a channel name is refused (S7, S8).

### What gets read
- **New:** each in-window message is stored as evidence, its text raw and untrusted (S12), with its edit time when edited (S13).
- **New:** system messages such as joins are skipped; bot messages are kept (S14).
- **New:** the token owner and every author are recorded (S15); an author profile that can't be read is a coverage gap, and the messages stay (S16).
- **New:** replies to threads started up to 30 days earlier are read (S17), without extra calls for quiet threads (S18), and a message at a window's edge is stored once (S19).

### What is never read
- **New:** a Slack Connect channel or a DM is refused before any history call (S9, S10); a channel the owner can't see reads "couldn't look: not_found_or_hidden" (S11).
- **New:** the token never appears in the database, the config or any output (S6).

### Failures
- **New:** a rate limit keeps the finished pages and reports when to retry (S20); a revoked token reads "couldn't look: auth" (S21) and a missing scope "couldn't look: forbidden" (S22).

## Walkthrough
1. The owner creates their own Slack app from the template, installs it and stores the token in the keychain (S23, S4).
2. `rastro check eng` reports authenticated, naming the keychain item (S1).
3. `rastro source add eng --connector slack --scope work --option channel=C0123ABCDE` saves the source (S7).
4. The first sync looks the channel up, reads its messages and authors, and reads coverage found (S12, S15).
5. An hour later the next sync reads a reply to a thread started two days ago (S17).

What can go wrong:
- The token has `chat:write`: the check refuses it (S2).
- The channel became a Slack Connect channel: nothing is read and coverage says forbidden (S9).
- Slack rate-limits the second page: the first page stays, the tick retries later (S20).

## Out of scope
- Commitments from Slack threads: the `chat-commitments` plan.
- Slack identity (token owner as `me`, accounts, bots): the `slack-identity` plan.
- A login command and token rotation: kept out until an admin-owned app needs them (round 2, Q2).
- DMs, group DMs, Slack Connect, file contents, search, and any write.
- Forgetting Slack evidence: slice 14.

## Risks
- **Token leakage:** a long-lived user token reads every approved channel; it is read at sync, never stored, and a test scans every output for it (S6).
- **Overbroad reads:** the check enforces the scope set (S2, S3) and the lookup refuses shared and direct conversations before any read (S9, S10).
- **Rate limits:** the 30-day lookback costs history pages on busy channels; a rate limit stops the run with a retry time instead of failing silently (S20).
- **Company policy:** a personal Slack app with history scopes may need the workspace admin's approval; ask before installing.

---

# Reference

## Build order
- T1 the crate, the manifest and template, the credential and the scope-enforcing check (regular · complex).
- T2 channel sync: the conversation guard, messages, authors, typed failures (regular · complex; after T1).
- T3 thread replies through the lookback and window edges, recorded in §50 (regular · complex; after T2).

Across plans: `slack-identity` builds after T2; `chat-commitments` builds after `slack-identity`.

## Decision log
- **D1 Native connector on the owner's own internal Slack app.** Rejected: reading Slack through the work companion; a shared app with a Rastro client id. Why: Slack's API terms bar an app distributed beyond its organization from keeping persistent copies or indexes and put its history reads on the lowest rate tier. Source: `docs/decisions/plugins.md` §50; user, 2026-11-21.
- **D2 Read-only user scopes only.** `channels:read`, `channels:history`, `groups:read`, `groups:history`, `users:read`. Rejected: a bot token, write scopes, search and DM scopes. Why: the connector only reads approved channels. Source: `docs/decisions/plugins.md` §50.
- **D3 One source per approved channel.** Each channel has its own coverage; Slack Connect channels and file contents are never read. Rejected: workspace-wide reads. Source: `docs/decisions/plugins.md` §50.
- **D4 First milestone is the credential check plus one-channel sync.** Rejected: check only (earlier draft). Why: a check alone stores nothing the owner can use. Source: user, round 1 Q3.
- **D5 Follow the Linear connector's pattern.** Token from `SLACK_USER_TOKEN`, else keychain item `rastro-slack`, never stored; test-only `RASTRO_SLACK_API_URL`, loopback only; no redirects; on HTTP 429 report `rate_limited` with `retry_at`. Rejected: sleeping on Retry-After, since the host kills a connector silent for 60 s (`crates/rastro-core/src/host/mod.rs:419`). Source: user, round 1 Q4; `crates/rastro-connector-linear/src/api.rs`.
- **D6 The check enforces the scope set.** It answers `rejected` when the token holds a write scope or lacks a required read scope, read from the `x-oauth-scopes` response header. Rejected: scopes by convention only. Source: user, round 1 Q5; Slack OAuth docs.
- **D7 Channel ids only, guarded before any history read.** `conversations.info` first: Slack Connect, externally shared, DM and group DM give "couldn't look: forbidden"; a private channel the owner can't see gives "couldn't look: not_found_or_hidden". Rejected: channel names as the key; id-prefix checks (new private channels also use `C` ids). Source: user, round 1 Q6; Slack `conversations.info` docs.
- **D8 Evidence shape.** Kinds `message`, `user`, `viewer`; locations `slack:message/<channel>/<ts>`, `slack:user/<id>`, `slack:viewer`; `/text` untrusted, `/user`, `/thread_ts`, `/subtype`, `/edited/ts` structured; update time is the edit time, else the post time; text stored raw. Rejected: rewriting mentions, since quotes must equal the stored bytes. Source: user, round 1 Q7; `docs/decisions/plugins.md` §25.
- **D9 Replies to older threads are read through a 30-day lookback.** Each run scans thread starts from the lookback and reads new replies of threads active in the window. Rejected: a coverage gap only, because with hour-long windows most replies would be missed. Source: user, round 1 Q8.
- **D10 The earlier check-only draft is deleted.** Source: user, round 1 Q2.
- **D11 No login command, rotation off.** The owner copies the app's user token into the keychain. Rejected: the pasted `rastro slack login` (PKCE, localhost redirect, rotation on, Slack commands in the core CLI). Why: PKCE and rotation are permanent on the app, refresh tokens die after 30 idle days, a CLI can't receive a custom-scheme callback, and Slack's docs conflict on plain-http localhost; a later plan adds a generic connector login only if an admin must own the app. Source: user, round 2 Q2; Slack docs (`using-pkce`, `using-token-rotation`, `tokens`).
- **D12 System messages skipped, bot messages kept, archived channels read normally.** Rejected: storing joins and topic changes. Why: they carry no commitment text; bots stay as evidence per `docs/decisions/knowledge.md` §27. Source: user, round 2 Q3.

## Code touched
- New `crates/rastro-connector-slack/` (manifest, template, `src/`, `tests/` with a scripted Slack API on loopback), modelled on `crates/rastro-connector-linear/` (`src/api.rs` credential and failure mapping; `tests/support/` home, keychain fake, `leaks()`).
- Workspace: `Cargo.toml` members; `scripts/affected-tests.test.sh` crate list; `scripts/crate-gate.sh` plugin role.
- Cross-connector check tests: `crates/rastro-cli/tests/source_check.rs`.
- Seams: `rastro source add --option`, `rastro check <source>` (`crates/rastro-core/src/host/check.rs`, 45 s, one stdout line), `rastro sync`, coverage reads.
- Spec: `docs/specs/evidence.md` (added requirements). Decision: `docs/decisions/plugins.md` §50 gains the lookback (T3).

## Interfaces

| Item | Value |
|---|---|
| Manifest | `id = "slack"`, `kinds = ["connector"]`, `protocol = 1`, `permissions.submit = ["message", "user", "viewer"]`, `scopes = ["personal", "work"]`, `env = ["SLACK_USER_TOKEN", "RASTRO_SLACK_API_URL"]`, `actions = []`, no `single_source` |
| Source option | `channel`, required, a Slack channel id |
| Check answer | `{result: authenticated \| missing \| rejected \| failed, credential, reason, fix}`; rejected names extra or missing scopes |
| Slack calls | `auth.test`, `conversations.info`, `conversations.history` (window plus 30-day lookback for thread starts), `conversations.replies` (threads with a reply in the window), `users.info` |
| Error mapping | `invalid_auth`, `not_authed`, `token_revoked`, `token_expired`, `account_inactive` → auth; `missing_scope` → forbidden; `channel_not_found`, `not_in_channel`, `thread_not_found` → not_found_or_hidden; 429 or `ratelimited` → rate_limited with `retry_at`; 5xx → network; bad JSON → protocol |
| Locations | `slack:message/<channel>/<ts>`, `slack:user/<id>`, `slack:viewer` |
| Template | YAML app manifest in the crate: user scopes = the five, no bot scopes, `token_rotation_enabled: false`, placeholder display name, setup steps as comments |
