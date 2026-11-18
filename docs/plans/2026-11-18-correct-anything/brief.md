a# Brief: Correct anything
Slug: `correct-anything`. Contract: `plan.yaml`. Lane: Plan, regular·complex. Owner writes change claim status across the projection, the queue and scoped reads, and reads gain a time dimension.

## Why
When `about me` shows a wrong claim, such as a stale assignee or a closed item still open, the owner can only remember that it is wrong, while agents keep reading it. The concept promises "the owner can confirm, fix or retract anything", and slices 8, 13 and 14 build on corrections, so this is the last M2 slice before Rastro acts.

## What Changes
| Area                            | Before                                                     | After                                                                                                                                                        | Scenarios     | Ticket |
| ------------------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------- | ------ |
| `claim confirm`                 | No way to say yes                                          | One `confirm` intent; the claim reads `confirmed` in `about`, `why` and its group                                                                            | S1, S3        | T1     |
| Revision check                  | None                                                       | A stale `--seen` revision is refused as a conflict; nothing recorded                                                                                         | S2            | T1     |
| `claims rebuild`                | Replays identity and queue intents                         | Also replays every correction; rebuilt = incremental                                                                                                         | S20           | T1     |
| Storage status                  | `observed` / `superseded`; scoped replay forces `observed` | Also `confirmed` / `retracted`; scoped replay keeps the real status                                                                                          | S5            | T1, T2 |
| `claim reject`                  | No retraction                                              | The claim leaves current reads in every scope that sees it; `why` shows `retracted`                                                                          | S4, S5        | T2     |
| `claim reject` on an owner link | n/a                                                        | Refused, naming `identity undo`                                                                                                                              | S6            | T2     |
| `claim edit`                    | No fix                                                     | One intent retracts the target and asserts the fixed object; out-of-range object refused                                                                     | S7, S8        | T3     |
| Automatic identity links        | Live while premises are observed                           | End when a premise is retracted, unless an owner link also justifies them                                                                                    | S9, S10       | T4     |
| Accepted commitments            | Each slot corrected alone                                  | Rejecting the anchor retracts the slots only it justifies                                                                                                    | S11           | T4     |
| `queue` known-first             | Accepted statements count as known                         | A retracted statement is not known; a matching proposal shows "earlier retracted"                                                                            | S12           | T4     |
| Correction binding              | n/a                                                        | Bound to the lifecycle fingerprint: a retraction lapses when its occurrence closes, survives corroboration; a confirmation goes stale when its evidence ends | S13, S14, S15 | T5     |
| `queue` questions               | Proposals and identity questions                           | Also one question per lapsed retraction or stale confirmation; yes retracts again                                                                            | S16, S17      | T6     |
| `claim undo`                    | n/a                                                        | Removes only the named intent; the latest valid intent sets the status                                                                                       | S18, S19      | T7     |
| MCP tool list                   | No write tools                                             | Still none; `about` / `why` schemas gain `as_of`                                                                                                             | S24           | T7     |
| `about` / `why` `as_of`         | n/a                                                        | Reconstructed from evidence and intents by then, under today's rules, and says so; names a source not read yet; MCP = CLI                                    | S21, S22, S23 | T8     |
| Switch day                      | n/a                                                        | The owner corrects their real `about me`; roadmap and README diagram marked done                                                                             | S25           | T9     |

## Examples
Illustrative only: ids are shortened, and output wording and versions are made up. The scenarios in `plan.yaml` are the contract. These examples are here to help us agree on the behaviour, and they come out once we do.

### 1. Confirm a claim you rely on (S1, S2)
```text
$ rastro about me
c_7f3a  ENG-88  code:state  open      observed  r_19b2
c_a201  ENG-88  assignee    you       observed  r_5d10

$ rastro claim confirm c_7f3a --seen r_19b2
confirmed c_7f3a (intent i_0c41, 2026-11-19 08:12)

$ rastro about me
c_7f3a  ENG-88  code:state  open      confirmed  r_8e77
```
A sync ran between your read and your write (S2):
```text
$ rastro claim confirm c_7f3a --seen r_19b2
error: c_7f3a changed since you saw it (now r_44c0). Read it again: rastro why c_7f3a
```
Nothing was recorded.

### 2. Retract a wrong claim, and see it lapse (S4, S13, S14, S16, S17)
Linear still says you own ENG-88, but Ana took it over on a call:
```text
$ rastro claim reject c_a201 --seen r_5d10
retracted c_a201 (intent i_0c52)

$ rastro about me            # ENG-88 assignee is gone
$ rastro why c_a201
status: retracted, by intent i_0c52 on 2026-11-19
```
- A GitHub PR that also names you as assignee arrives later. The claim stays hidden, because corroboration does not undo a retraction (S14).
- Linear reassigns ENG-88 to Ana and then back to you. The occurrence you retracted closes, so the retraction lapses (S13):
```text
$ rastro about me
c_a201  ENG-88  assignee  you  observed · earlier retracted

$ rastro queue
q_91d3  question  You retracted "ENG-88 assignee: you" on 2026-11-19 (i_0c52).
                  Linear says it again. Still wrong?  yes / no

$ rastro queue answer q_91d3 yes       # hidden again (S17)
```

### 3. Fix a value instead of retracting it (S7, S8)
```text
$ rastro claim edit c_b310 --seen r_7a02 --object "Ana Souza"
retracted c_b310, asserted c_e9f1 (intent i_0c60)

$ rastro why c_e9f1
name: Ana Souza   justified by: your edit i_0c60 (only)

$ rastro claim edit c_d4c2 --seen r_1101 --object "next-ish"
error: std:due takes a date; "next-ish" is not one. Nothing recorded.
```

### 4. Carry-through (S9, S10, S11, S12)
- **Identity:** your Linear account was linked to you automatically through the verified email `team@…`, which is a shared alias. You reject that email claim, and the automatic link is gone (S9). If you had also run `identity link` on it, the link stays, now justified only by your link (S10).
- **Commitment:** yesterday you accepted "send the rollout plan to Ana by Friday". You reject its anchor, and the `std:owes`, `std:owed_to` and `std:due` slots go with it (S11). Next week a meeting repeats the promise, and the queue shows it again (S12):
```text
q_7c20  proposal  send the rollout plan to Ana by Friday   · earlier retracted
```

### 5. Undo (S18, S19)
```text
$ rastro claim confirm c_7f3a --seen r_19b2   # i_1
$ rastro claim reject  c_7f3a --seen r_8e77   # i_2 → retracted
$ rastro claim undo i_2                       # → confirmed again; why shows i_2 undone
```

### 6. As of a past time (S21, S22, S23)
```text
$ rastro about me --as-of 2026-11-10T09:00Z
Reconstructed from evidence captured and intents issued by 2026-11-10 09:00 UTC,
with today's rules (rastro-extractor-linear 0.4.0, vocabulary std 3).
meetings: not read yet at this time
c_a201  ENG-88  assignee  you  observed        # your 11-19 retraction did not exist yet
```
An agent calling the MCP `about` tool with `{"entity": "me", "as_of": "2026-11-10T09:00Z"}` gets the same claims (S23). Its tool list has no `claim.*` write (S24).

## Capabilities
New spec: `docs/specs/corrections.md` (11 added requirements, plus one switch-day requirement). `claims.md`, `identity.md` and `queue.md` are unchanged as specs (D13). Their behaviour changes are written as corrections requirements.

## Impact
- Storage: `crates/rastro-storage-sqlite/src/{lifecycle.rs, scoped.rs, knowledge.rs, automatic.rs}` and a new migration widening `claim.status`.
- Core: `crates/rastro-core/src/identity.rs` (`IntentKind`, `Targets::Claim`/`ClaimSet` validity, undo validation at :914), `claims.rs` (revision, fingerprint), `knowledge.rs` (about/why views, groups), `queue/` (known-first, new question kind).
- Surfaces: `crates/rastro-operations/src/{catalog.rs, requests.rs, results.rs}` (four new CLI-only slots, `as_of` on `AboutRequest`/`WhyRequest`), `crates/rastro-cli/src/{main.rs, mcp.rs}`, `schema/operations-v2.schema.json`.
- Tests: `crates/rastro-cli/tests/support/` drives the binary. New suites are `corrections.rs`, `corrections_carry.rs`, `corrections_lapse.rs`, `corrections_undo.rs` and `as_of.rs`. Fixtures follow `queue.rs` (fake adapter, `Env`) and `about_why.rs`.
- Docs: ADR 0013 (lifecycle fingerprint), the corrections spec, roadmap and README diagram at switch day.

## Context
- Owner intents already exist (ADR 0010): insert-only, with `undo_of`/`supersedes`, `intent_justification`, and replay at rebuild. The `Claim` and `ClaimSet` target shapes are reserved, and no kind uses them yet.
- `about` returns each claim's id and `revision`. A `why` request takes a claim id. A group's revision hashes its members' (id, revision) pairs.
- The only claim-from-claim derivation today is the automatic identity link (`automatic_justification`, premises `verified_email`/`token_owner`). An accepted commitment's claims have the accept intent as their only justification.
- The queue recomputes known-first and identity questions on every read. Outcomes come from intents and shown reports, never from a second table.
- There is no local store in the repo, so this plan has no real counts. The queue was empty after slice 6's switch day (14 items decided on 2026-11-18). Carry-through to pending proposals is tested on fixtures.
- The workspace-specs plan (2026-11-17) is not built yet. Tests still tag each requirement by its title, so the spec gate covers them once it lands.

## Goals / Non-Goals
Goals: confirm, retract and fix any projection claim. Carry-through to derived claims, commitment slots and the queue. Lapse, staleness and the Ask. Undo. `as_of` on `about`/`why`.

Non-goals:
- Correcting hypothesis commitments (the queue's `reject` covers them).
- Flagging Action runs (slice 8's outbox, D11).
- An event bus.
- `as_of` on `search`, `commitments` or `queue`.
- Valid-time ("true in the world at t") queries.
- Rules such as "never show again".
- Correction writes over MCP.
- Undoing `create_person` or `cannot_link` (unchanged from ADR 0010).

## Decisions
- D1 A correction is an owner intent (insert-only, outside the rebuildable projection, exported), over editing claim rows: the projection is rebuilt from evidence, confirmations, rules and intents (`decisions/knowledge.md` §12, §42, §53; ADR 0010 envelope).
- D2 Corrections target the reserved `Claim` and `ClaimSet` shapes (id, revision seen, statement snapshot, lifecycle binding, anchor), over a new envelope or schema rewrite (ADR 0010 "Reserved target shapes"; `crates/rastro-core/src/identity.rs` `Targets`).
- D3 An intent binds to the lifecycle state the owner saw, not the full revision, so extra corroboration does not expire a yes, over binding to every occurrence (knowledge §42; renderers §8).
- D4 An intent naming a claim whose lifecycle state changed since the owner saw it is refused as a conflict, over a silent overwrite (renderers §5).
- D5 A retraction hides the statement while its bound state holds; when the state changes it lapses, the claim returns with its evidence-derived status marked "earlier retracted", and an Ask is raised. A confirmation survives a provenance change but goes stale, shown with its history. Over a retraction that holds forever (knowledge §42).
- D6 A new yes on a retracted claim reverses it, latest intent wins; undo removes only that intent's contribution; "never show again" is a Rule, not an intent (knowledge §42).
- D7 Precedence: valid retraction > valid confirmation > deterministic observed > hypothesis > superseded, over evidence beating the owner (knowledge §42).
- D8 Retracting a premise ends only the derivations that rest on it; a claim with an independent justification survives; no blanket deletion of descendants; cycles rejected. Today the only claim-from-claim derivation is the automatic identity link (`automatic_justification`). Over cascading deletes (knowledge §38; ADR 0010 "Inference storage").
- D9 An intent applies in every scope that can see a justification of its target; the issuer must see the target, over per-scope corrections (knowledge §42, §44; ADR 0010 Visibility).
- D10 "As of t" is a reconstruction: recompute from evidence captured by t under today's pinned rules, and the answer says so, over stored answer receipts (knowledge §47; claims R24 publication row).
- D11 Carry-through reaches derived claims and pending proposals; nothing executes on it. Flagging Action runs waits for slice 8's durable outbox, over emitting action-affecting events now (knowledge §10, §43).
- D12 Scoped replay replays the real status, over today's forced `observed` in `scoped.rs`, so a scoped read never shows a retracted claim (knowledge §48 entry condition).
- D13 The plan's spec is a new `docs/specs/corrections.md` with added requirements only, over modifying `claims.md`, whose requirements are `R<n>` bullets the archive cannot target by title (gather: spec formats).
- D14 Corrections reuse the intent kinds `confirm`, `reject` (retract) and `edit` (fix) on a `Claim` target, over new `retract`/`fix` kinds and a glossary change: renderers §1 and the glossary already name them (owner, grill Q1).
- D15 `edit` is one intent that retracts the target and mints an owner-asserted claim with the same subject and predicate, a new object, and the edit intent as its only justification. Only the object is fixable. This is over superseding the object in place, and follows how an accepted proposal mints its claim (knowledge §73; owner, Q2).
- D16 Every claim in the projection is correctable: observed from structured evidence, intent-backed, automatic links. Hypothesis commitments are not, over making them correctable too: `queue reject` already handles them (knowledge §70; owner, Q3).
- D17 `reject` on a held_by/same_as claim that an owner `link` made is refused and names `identity undo`, over two paths to one end. An automatic link can be rejected; the rejection holds while its premises hold and records no cannot-link (ADR 0010 undo; owner, Q4).
- D18 Rejecting an accepted commitment's anchor retracts every slot that only that anchor justifies, over one reject per slot. Slots can still be rejected one at a time (knowledge §42 claim sets; owner, Q5).
- D19 A retracted claim's statement no longer counts as known, so new evidence can propose it again, marked "earlier retracted". Suppressing it for good is a Rule (D6), over treating retracted statements as known (knowledge §72; owner, Q6).
- D20 Carry-through to pending proposals happens on the queue read, which already recomputes known-first and identity questions on every read, over an event bus in this slice (knowledge §43; queue spec; owner, Q7).
- D21 A lapsed retraction or a stale confirmation raises an Ask as a queue question in this slice, over shipping only the marks (knowledge §42; owner, Q8).
- D22 "As of t" applies evidence captured by t and intents issued by t, over evidence by t plus today's intents (knowledge §47; owner, Q9).
- D23 Writes are CLI-only: `rastro claim confirm|reject|edit <claim-id> --seen <revision>` and `rastro claim undo <intent-id>`. `about` and `why` take `as_of` on CLI and MCP. MCP stays read-only, and `search` and `commitments` get no as-of. Over MCP writes (surfaces §37; ADR 0007; owner, Q10).
- D24 The lifecycle fingerprint is the hash of the claim id, the current occurrence of each live justification, and its `conflicts_with`/`supersedes` links. The renderer revision stays the fuller hash with the occurrence history, so new corroboration changes the revision and leaves the fingerprint alone. Recorded as ADR 0013, over binding to the revision (knowledge §42, §48; ADR 0010 "Deferred to slice 7"; owner, Q11).
- D25 Switch day: the morning after the last code ticket merges, the owner rejects or edits at least one wrong claim in `about me` and confirms one claim they rely on, over a fixture-only demo (roadmap preamble; owner, Q12).
- D26 In a merged Person's group, members rank by the D7 precedence and a retracted member is left out, over ranking by recency. This follows from D7 and closes ADR 0010's deferred "how non-observed statuses rank in a group" (ADR 0010; D7).
- D27 Accepting a proposal also asserts the commitment's anchor: a `rastro:type std:Commitment` claim on `std:commitment/<item id>`, justified by the accept intent. The commitment's slot claims rest on the anchor, so rejecting it ends every slot that only it justifies, and a slot with its own observed justification stays. Rebuild replays accepts, so commitments accepted earlier gain their anchor. Over retracting the commitment as a `ClaimSet`, which needs a new CLI form (D18; review of the carry-through PR found no anchor claim in the code; owner, 2026-11-18).

## Risks / Trade-offs
- **Migration on the owner's store.** Widening the `claim.status` CHECK rewrites a table in a store with thousands of claims (2,248 on 2026-11-05, knowledge §50). Mitigation: T1's migration test (`tests/migrations.rs`) runs it over a populated fixture, and the projection stays rebuildable from evidence and intents.
- **Scoped leak.** Until T2 lands, a scoped read could show a retracted claim. T1 fixes scoped replay before any `reject` kind exists, and S5 pins it.
- **Two paths to unlink.** `reject` and `identity undo` could both end an owner link. S6 refuses one so that only one path exists.
- **Reconstruction is not history.** `as_of` re-runs today's rules, so an answer can differ from what an agent was shown then. The result says so (S21), and no receipts are stored (D10).
- **Chain length.** T1→T2→T3 run in series. T4/T7 and then T5/T8 run in parallel on disjoint paths. A slip in T1 delays everything, which is why T1 holds only confirm and the status groundwork.
- **Privacy.** A correction applies in every scope that sees its target, and the issuer must see it (D9). Hidden data can suppress a claim but never create one.
- **Rollback.** Intents are insert-only. A bad correction is undone with `claim undo` (T7), never by deleting rows.

## Open Questions
Assumed without asking:
- Answering the Ask "no" records the answer and lets the claim stand with its evidence status, the opposite of S17. No scenario pins the "no" path beyond the queue's existing "One intent per answer" requirement.
- `claim undo` of an `edit` removes both the retraction and the asserted claim, since the asserted claim's only justification is that intent.
