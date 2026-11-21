# Know who is who in Slack
Lane: Plan, regular and complex: a new extractor and an owner-approved vocabulary namespace that feed identity.

A synced Slack channel records authors and the token owner, but nothing yet says who they are. This plan adds a Slack extractor bound to the Slack connector: the token owner becomes the owner's Person, every author becomes a Slack account with its names, and bots never link to anyone. Without email (the read scopes give none), Slack accounts link to people only explicitly. One ticket, after the Slack connector's channel sync.

## Your calls
None.

## What changes

### The owner's identity
- **New:** after a Slack sync, the token owner's Slack account is held by `me` with a token-owner justification (S1).

### Other people
- **New:** each author becomes a Slack account with its name and display-name aliases (S2).
- **New:** a bot author is typed as a Slack bot and no Person holds it (S3), and a bot sharing a name with another account is never offered as a link candidate (S4).

## Walkthrough
1. `me` exists; the owner syncs a Slack channel with messages from Ana and a deploy bot (S1).
2. The owner's Slack account is held by `me` (S1).
3. Ana becomes a Slack account with her name and display name as aliases (S2).
4. The deploy bot becomes a Slack bot account held by nobody (S3).

What can go wrong:
- The deploy bot is named like a GitHub account: no link candidate appears (S4).

## Out of scope
- Linking Slack accounts by email: the scopes give no email (round 2, Q1).
- Commitments from Slack threads: the `chat-commitments` plan.

## Risks
- **Wrong self-link:** the token owner is taken from Slack's own answer for the token, not from a name, so a namesake can't become `me`.
- **Namespace growth:** `slack` is a new vocabulary namespace; the owner approved it in this plan's grill, recorded in `docs/decisions/plugins.md`.

---

# Reference

## Build order
- T1 the Slack extractor and `slack` vocabulary bound as the connector's default extractor, with the namespace approval recorded (regular · complex).

Across plans: build after `native-slack` T2, which records the viewer and authors.

## Decision log
- **D1 Slack identity ships with the connector, mirroring Linear's.** Token owner holds `me`; authors are `slack:user/<id>` accounts with name aliases; app and bot users are `slack:Bot` and never link. Rejected: an evidence-only connector. Why: chat commitments need speakers resolved by account. Source: user, round 2 Q1; `docs/specs/identity.md` "The Linear key owner holds me", "An app user is a bot and never links".
- **D2 The `slack` namespace is owner-approved.** Rejected: reusing the `tracker` namespace. Why: §47 requires the owner's approval for any new namespace; the round 2 yes is that approval. Source: user, round 2 Q1; `docs/decisions/plugins.md` §47.
- **D3 No email-based links.** Rejected: adding `users:read.email`. Why: §50 fixes the read scopes; linking stays explicit. Source: user, round 2 Q1; Slack `users.info` docs.

## Code touched
- New `crates/rastro-extractor-slack/` (manifest, `vocabulary.toml`, `src/`, `tests/`), modelled on `crates/rastro-extractor-linear/` (viewer to `rastro:token_owner`, `/app` to Bot, canonical location rule).
- `crates/rastro-connector-slack/rastro-plugin.toml` gains `default_extractor`.
- Workspace: `Cargo.toml`, `scripts/affected-tests.test.sh`.
- Spec: `docs/specs/identity.md` (added requirements). Decision: `docs/decisions/plugins.md` (namespace approval).

## Interfaces

| Item | Value |
|---|---|
| Namespace | `slack`: types `Account`, `Bot` |
| Inputs | structured observations at `slack:viewer` (`/id`) and `slack:user/<id>` (`/id`, `/name`, `/real_name`, `/profile/display_name`, `/is_bot`, `/deleted`) |
| Claims | viewer `/id` → `rastro:token_owner`; account aliases from name and display name; `/is_bot` true → `slack:Bot` |
