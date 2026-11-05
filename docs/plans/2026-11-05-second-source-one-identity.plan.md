# Plan: A second source, one identity

Slug: `second-source-one-identity`. Living spec: `docs/specs/identity.md` (new; `docs/specs/surfaces.md` and `docs/specs/evidence.md` amended). Status: APPROVED 2026-11-05.

## Why now
Roadmap slice 4 closes M1. Rastro has one connector (GitHub), and on the owner's data the only cross-source entities (FBisca, preston-ai-bot) are already one entity without a merge (knowledge §24). No Person, no owner intent and no link exist in code. Without one identity per thing the assistant contradicts itself (concept §1). Slices 5–7 (commitments, the queue, corrections) need Persons and owner intents to exist first.

The kalios data has one human and no email, so a second source there would join nobody but the owner. The owner's Heritage Linear workspace and the `heritageholdings/svc-platform` repository share a whole team, so they exercise real links, real fuzzy candidates and real non-matches.

## Sources
| Source | What it settles |
|---|---|
| Owner, 2026-11-05 session: Q1–Q25 accepted, plan approved (Q1 a; Q2, Q4–Q9 as recommended; Q3 reframed as the slice-7-shaped intent foundation; Q10–Q22 as recommended) | D1–D22 |
| Codex design review of the intent foundation, 2026-11-05 (agree with changes; all changes accepted by the lead, gaps put to the owner as Q19–Q22) | D6–D11 |
| Gather report and real DB (immutable copy), 2026-11-05 | 3 sources (`kalios`/work unregistered, `kalios-mobile` never synced, `kalios-api`); 719 entities, 2248 claims all observed, 3 accounts (1 human, 2 bots), 3 login aliases, 0 emails, 0 claim links; 2 entities seen in two sources, already one entity each |
| Fleet brain, 2026-11-05 | Heritage Linear: 4710 issues across 142 projects; GitHub `heritageholdings/svc-platform`: 581 PRs; no kalios item in either |
| `docs/concept.md` §1; `docs/roadmap.md` slice 4 | One identity per thing; the slice's outcome |
| `docs/GLOSSARY.md` Person, Entity, Alias, Merge, Intent | An account is linked to a Person, never merged; a merge is a reversible claim between entities of one kind |
| `docs/decisions/knowledge.md` §4, §12, §24, §33–36, §42, §44, §46, §48, §50–52 | Exact links automatic, fuzzy asks, names never merge; one identifier is one entity; claim ids never re-keyed; merge is an overlay claim, undo records a cannot-link; grouping on read; Person minting and email rules; intents (slice 7 shape); visible-only scoped answers; precious inputs; rebuild carry-over |
| `docs/decisions/vocabulary.md` §2, §8, §14–15, §20–23 | Person is core; aliases and types are claims; `rastro:held_by`, `rastro:same_as`; shared account may link to two Persons; reused login is ambiguous |
| `docs/decisions/surfaces.md` §4, §23, §27, §31–36; `docs/decisions/renderers.md` §7–8 (2026-11-05) | `about` forms; agent writes go through MCP; set targets with member revisions |
| `docs/decisions/plugins.md` §1, §15, §18–19 | The core validates everything; one extractor binding per source |
| ADR 0008, ADR 0009 (with amendments) | Extractor protocol; projection tables, shadow-swap rebuild; `held_by`/`same_as` reserved for the slice-4 ADR |
| `docs/open-questions.md` #10 | An unresolved entity looks like "nothing known" (decided here, D20) |
| `docs/specs/claims.md`, `surfaces.md`, `evidence.md` | Current alias, type, scope and registration behaviour this plan extends |

## Decisions
| # | Choice | Rejected | Why | Source |
|---|---|---|---|---|
| D1 | Second source: Linear (Heritage workspace). The owner also adds `heritageholdings/svc-platform` as a GitHub `work` source | Slack; an address book; mail | Same team in both; structured API with member emails; feeds slice 5 | Owner Q1 |
| D2 | Persons exist only through an owner intent (`rastro identity me`, `rastro identity person create`, or a yes to a candidate). An account links on its own to an *existing* Person only through an equal verified email or a token-owner observation (D13–D14). Everything else is a fuzzy candidate. Roadmap wording becomes "exact identifiers link on their own" | A Person per verified email (amends §36); nothing automatic | Matches knowledge §36 and the glossary | Owner Q2 |
| D3 | Writes are CLI only (`rastro identity …`). MCP stays read-only | MCP write operations | No new write surface for agents in this slice | Owner Q4 |
| D4 | Fuzzy candidates are computed on read, never stored. The owner's yes or no is stored as an intent; an answered pair never returns | Stored hypothesis claims; no fuzzy matching | Hypotheses are slice 5, the queue slice 6 | Owner Q5 |
| D5 | Core type `rastro:Person`. Account → Person through `rastro:held_by`; Person ↔ Person merge through `rastro:same_as`. Accounts never merge with each other. Undoing a link or merge records a cannot-link | A vocabulary-plugin Person; account-to-account merge | Vocabulary §2, §14; glossary | Owner Q6, Q7 |
| D6 | Build the owner-intent foundation now in the slice-7 shape, with only identity kinds enabled: `create_person`, `name`, `link`, `cannot_link`, `same_as`, `undo`. Slice 7 adds claim and claim-set kinds, the lifecycle binding, lapse/stale and precedence without replacing the table | A minimal identity-only table; pulling all of slice 7 forward | Built once, right; full slice 7 needs the hypothesis status and the queue | Owner Q3; Codex review |
| D7 | One insert-only `intent` table: `id` (portable, retries reuse it), `append_seq` (unique acceptance order), `format_version`, `kind` (validated by the core, no SQL enum), `owner_id`, `issuer_context`, `issued_at`, `domain_key` (indexed, not unique), `targets_json` and `payload_json` (versioned, tagged), `undo_of`, `supersedes` (self-references). No foreign key to entities, claims or occurrences. No update or delete outside a future controlled forget path. Target shapes reserved now: entity (id, identifier, role); pair (roles; sorted only for symmetric relations); claim and claim set (id, reviewed revision, statement snapshot, nullable versioned lifecycle binding, anchor) | Per-kind tables; foreign keys to projection rows | Slice 7 extends without a schema rewrite; rebuild never breaks references | Codex review |
| D8 | Resolution is per `domain_key`, not "latest wins per target": creating, naming, linking and merging never overwrite each other. An undo targets one earlier intent (`undo_of`) and removes only its contribution. Undoing a link or merge also records a cannot-link. An explicit link or merge by the owner is the only thing that clears a cannot-link | Latest intent wins per target | Codex review point 3; knowledge §34 | Codex review |
| D9 | Identity intents target entities, not a claim's lifecycle state, so they carry no lifecycle binding and never lapse. The slice-4 ADR records this exemption from §42 | Binding identity intents to a fingerprint | The fingerprint is defined in slice 7 | Codex review |
| D10 | Intent-backed claims (`rastro:Person` type, `name` alias, `held_by`, `same_as`) keep status `observed` with a justification of kind `owner_intent`; `why` shows the intent. The `confirmed` status and the scoped-replay fix stay in slice 7 (knowledge §48 entry condition) | A `confirmed` status now | Keeps the §48 entry condition intact | Owner Q20 |
| D11 | Rebuild keeps the intent table outside the swapped tables, replays intents after evidence replay and carry-over, and resolves overlays before publication. The scoped-read cache is invalidated by an intent write | Intents inside the disposable projection | Precious input; knowledge §50–52 | Codex review |
| D12 | A Person is visible in a scope that can see at least one of its linked accounts; `me` is visible in every scope. A link or merge is visible only where both ends are. A cannot-link applies in every scope. The issuer must see the target at issue time; the recorded issuing context is audit data, not lasting authorization | Visible wherever either end is | Knowledge §42, §44 | Owner Q9, Q19; Codex review |
| D13 | A verified email is: a Linear workspace member's email, or one of the token owner's own verified GitHub emails (`/user/emails`, `verified: true`). A GitHub profile email is self-asserted and feeds only fuzzy candidates. Emails are stored as `email` alias claims in the source's namespace (`linear:email/…`, `github:email/…`) and compared across namespaces by the identity rule, whole address with ASCII case-folding only, no provider normalisation (no dot or plus rules) | Comparing raw strings; provider normalisation | Knowledge §36 forbids provider normalisation | Owner Q11; owner Q23 (comparison formula) |
| D14 | Automatic links are not intents. Verified-email and token-owner observations are evidence; links are derived from them on every projection build and rebuild (never from live credentials), with intervals: an email link holds while that email is observed on both accounts over an overlapping interval; a token-owner link holds while the token belongs to that account. A cannot-link vetoes an automatic link, including through a `same_as` chain. Equal emails over non-overlapping intervals never link | Storing automatic links as intents | Rebuildable from evidence; knowledge §36 | Owner Q16; Codex review |
| D15 | `rastro identity me` creates the owner's Person (one intent; a second run returns the same Person). Each connector reports the account its token belongs to; that account links to `me` on its own | Creating `me` implicitly at first sync | `me` must come from an owner intent (§36) | Owner Q12 |
| D16 | A Person's id is `rastro:person/<random id>` minted by its intent. Its name is a `name` alias (`rastro:name/<name>`); a name two Persons share returns the existing ambiguous-alias error. `me` is a reserved entity form | An id from the name or email | Knowledge §36 | Owner Q13 |
| D17 | Fuzzy candidate signals: equal full name after case and accent folding; a Linear email whose local part equals a GitHub login. Pairs within one namespace, bots, and answered pairs are never candidates. A yes links both accounts to the Person one of them already has, or creates a Person (named from the candidate) and links both; a no records a cannot-link between the two accounts | Display-name substring or edit-distance matching | Few, explainable signals; names alone never link | Owner Q14; owner Q24 (yes/no effects) |
| D18 | `about(person)` lists the Person, its linked accounts each with the reason (verified email, token owner, owner intent), and the claims of those accounts in one view; each claim cites its own account; equivalent claims group per knowledge §35 and count once toward `limit`. `about(account)` shows its `held_by` Person. The existing work budget applies; exceeding it fails honestly | Showing only the Person's own claims | Owner Q15; knowledge §35 | Owner Q15 |
| D19 | Linear connector reads workspace users (id, name, display name, email, active) and issues the API-key user created, is assigned or subscribes to (identifier, title, description, state, creator, assignee, team, timestamps). Comments and issue-to-PR links wait for slice 5. Its vocabulary is `tracker` (`tracker:Account`, `tracker:Issue`); identifiers `linear:user/<uuid>`, `linear:issue/<uuid>` | Users only; all workspace issues | Mirrors GitHub's "involves me"; slice 5 needs issues | Owner Q10; owner Q25 (namespace and identifiers) |
| D20 | `about` on a well-formed identifier that resolves to nothing visible returns `resolved: false`; a hidden-only entity looks the same. Closes open question #10 | Leaving it open | Agents read empty as "nothing known" | Owner Q8 |
| D21 | Each connector manifest declares its source options (name, required, pattern, help). `source add --connector <id> --option key=value` validates against it; GitHub's `--repo` stays as a shorthand and its check moves into the GitHub manifest | A Linear-specific CLI flag | Q17; plugins §1 | Owner Q17 |
| D22 | The Linear API key is read at sync from `LINEAR_API_KEY` or the keychain item `rastro-linear`, like the GitHub token; never stored in the database or config. Every owner intent is a precious input and is exported in slice 13 (amends knowledge §12, §46); derived automatic links are not exported. Intents keep copied names and identifiers until slice 14 adds a controlled purge that leaves a marker (accepted limit) | Storing the key; leaving intents out of §12 | Q18, Q21, Q22 | Owner Q18, Q21, Q22 |
| D23 | T2 may change `Cargo.lock` (added to its Paths) for a direct `regex` dependency in the plugin SDK, used to check manifest-declared option patterns; `regex` 1.13.1 is already in the lock transitively | A hand-written pattern matcher | Build-level scope fix, recorded by the lead during the build (2026-11-05) | Lead, T2 build |
| D24 | Source options are checked strictly: a manifest that declares no options refuses any option; a connector not installed yet may be added, but its options are checked at sync before binding (failing as `protocol`); GitHub falls back to the bundled declaration when the installed manifest declares none; a stored config is checked the same way at sync. T2 may change `crates/rastro-core/src/runs/mod.rs` for the sync-time check, and record this in `docs/decisions/plugins.md` and the `source.add` schema addition in `docs/decisions/surfaces.md` (all added to its Paths) | Unchecked when no options are declared or no manifest is found | plugins §1 (the core validates everything) and D21; build-level, recorded by the lead during review (2026-11-05); confirmed by the owner (2026-11-05) | Lead, T2 review; owner Q1 (2026-11-05) |
| D25 | Only an account can be held by a Person: core declares the supertype `rastro:Account`; a vocabulary declares which of its types are accounts (`code:Account`, `tracker:Account`); a link, owner or automatic, whose account side isn't typed as an account is refused | Any non-Person entity may be linked | A PR or repository held by a Person is nonsense, and automatic links would inherit the hole; core can't name plugin vocabularies | Owner Q2 (2026-11-05), from the T3 review |
| D26 | An undone link or merge, and a Person's older name, stay in the projection as `superseded`, with the interval closed at the undo (or newer name) time; an old name still resolves as a historical alias | Removing the claim, leaving history only in `rastro identity history` | knowledge §34 (undone without deleting history), §26/§37; vocabulary §20 | Owner Q3 (2026-11-05), from the T3 review |
| D27 | Creating a Person can't be undone until slice 14's controlled removal; a mistaken Person is merged into the right one | An undo of `create_person` | Merge already covers the mistake | Owner Q4 (2026-11-05) |
| D28 | An intent's `issuer_context` records the issuing surface's actual configured scopes, not `[]` meaning all | `[]` as "all scopes" | ADR 0010: effective scopes are audit data | Owner Q5 (2026-11-05), from the T3 review |
| D29 | A retried `rastro identity person create` mints a new Person; a duplicate is merged. Revisit when an agent surface can issue intents | Reusing the intent id across CLI retries | No retry path exists in the CLI today | Owner Q6 (2026-11-05) |
| D30 | ADR 0010's build notes are confirmed: newest name wins; an account–account cannot-link blocks automatic links, directly or through a `same_as` chain; the drafted `domain_key` formats; atomic row validation including the snapshot hash; export keeps acceptance order; a cleared cannot-link stays in history marked reversed | Leaving them as notes | Built in T3 as drafted | Owner Q7–Q9 (2026-11-05) |
| D31 | Linear app (integration) users are typed `tracker:Bot`, like GitHub's `code:Bot`: never held by a Person, never a candidate | Every Linear user as `tracker:Account` | vocabulary: an account is not an integration acting without a user; D17 | Owner Q10 (2026-11-05), from the T4 review |
| D32 | A Linear guest's email is not verified: it never makes an automatic link, though it may still feed a fuzzy candidate | Treating guests like members | D13 says a workspace member's email | Owner Q11 (2026-11-05), from the T4 review |
| D33 | One Linear workspace for now: a second source for the `linear` connector is refused with an error naming the existing one; per-source keys wait until needed | A per-source keychain item now | One key (`LINEAR_API_KEY`, keychain `rastro-linear`) would read the same workspace into another scope | Owner Q12 (2026-11-05), from the T4 review |
| D34 | A token's owner is recorded as kind `viewer` at the per-source location `<namespace>:viewer` (field `/id`), one slot a different owner replaces; the GitHub connector records its token owner the same way and T6 derives both token-owner links from it | Per-connector shapes | Linear's connector already records it so (T4) | Owner Q13 (2026-11-05), from the T4 review |

## Scenarios
**Source registration**
- **S1** WHEN `rastro source add --connector <id>` is given an option its manifest doesn't declare, or omits a required one, THEN it's refused with an error naming the declared options, and nothing is saved.
- **S2** WHEN `rastro source add --connector github --repo owner/name` runs, THEN the source is saved as before; WHEN the repo doesn't match the GitHub manifest's pattern, THEN the error comes from that declared option.

**Owner intents and Persons**
- **S3** WHEN `rastro identity me` runs, THEN a `rastro:person/<id>` entity typed `rastro:Person` exists and `about me` resolves to it; WHEN it runs again, THEN it returns the same Person and records no new intent.
- **S4** WHEN `rastro identity person create --name "Ana Silva"` runs twice, THEN two distinct Persons exist and `about "rastro:name/Ana Silva"` returns the ambiguous-alias error listing both.
- **S5** WHEN the owner links `github:login/<x>` to a Person, THEN `about github:login/<x>` shows `rastro:held_by` that Person, and `why` on the link shows status `observed` with an `owner_intent` justification giving the intent's id, time and issuer.
- **S6** WHEN the owner undoes that link, THEN the `held_by` claim is gone, a cannot-link for the pair is recorded, and `rastro identity history <entity>` lists the link, undo and cannot-link in acceptance order.
- **S7** WHEN the owner merges two Persons, THEN `about` on either shows the claims of both Persons' accounts with equivalent claims grouped; WHEN the merge is undone, THEN they separate and a cannot-link is recorded.
- **S8** WHEN an intent is invalid (a `held_by` from a Person, a merge of two accounts or of an entity with itself, a target that doesn't exist, an undo of a missing or already-undone intent), THEN it's refused with an operation error naming the reason, and no intent is stored.
- **S9** WHEN an intent is issued from a context whose scopes can't see its target, THEN it's refused (storage seam).
- **S10** WHEN `claims rebuild` runs, including with an extractor missing so a pair is carried over, THEN every intent-backed Person, name, link and merge reappears identically, and the intent table is unchanged (rows and order).
- **S11** WHEN an intent is written after a scoped read was cached, THEN the next scoped read reflects it.

**Automatic links**
- **S12** WHEN the GitHub connector syncs, THEN evidence records the token owner's account, its verified emails (unverified ones excluded) and each author account's profile name, and `about` on the token owner's account shows its `github:email/…` aliases.
- **S13** WHEN the GitHub token can't read `/user/emails`, THEN coverage says the emails couldn't be read, and the sync still succeeds.
- **S14** WHEN `me` exists and a sync records the GitHub token owner, THEN that account is `held_by` `me` with a token-owner justification, with no owner command.
- **S15** WHEN account B has a verified email equal (ASCII case-folded) to a verified email of an account already linked to Person P, over an overlapping interval, THEN B is `held_by` P, justified by both email observations; WHEN no linked account has that email, THEN no link and no Person is created.
- **S16a** WHEN a matching email belongs to a Linear guest, THEN no automatic link is made.
- **S16** WHEN the equal emails were observed only over non-overlapping intervals, THEN no link exists.
- **S17** WHEN a cannot-link exists between B and P (directly or through a `same_as` chain), THEN the automatic link isn't made; WHEN the owner then links them explicitly, THEN the link exists.
- **S18** WHEN the email is no longer observed on B, THEN the link's interval closes and `about` shows it as historical.
- **S19** WHEN `me` exists and a Linear sync records the API key's user, THEN that Linear account is `held_by` `me` with a token-owner justification.

**Linear source**
- **S20** WHEN a Linear source syncs against the scripted API, THEN users become `linear:user/<uuid>` entities typed `tracker:Account` with `email` and `name` aliases and their active state, the viewer's involved issues become `tracker:Issue` entities with creator and assignee claims, and `rastro search` finds issue text with citations.
- **S21** WHEN a Linear user is deactivated, THEN its account shows it, and its existing links remain, marked deactivated.
- **S22** WHEN no Linear API key is available, THEN the pair fails with an auth failure shown in coverage, and the key never appears in the database or config.

**Fuzzy candidates**
- **S23** WHEN a GitHub account and a Linear account have the same full name after case and accent folding and aren't linked to one Person, THEN `rastro identity candidates` lists the pair with reason `same name`.
- **S24** WHEN a Linear email's local part equals a GitHub login, THEN the pair is listed with reason `email matches login`.
- **S25** WHEN the owner answers yes to a candidate, THEN both accounts are `held_by` one Person (the existing one, or a new one named from the candidate) and the pair no longer appears.
- **S26** WHEN the owner answers no, THEN a cannot-link is recorded, the pair never appears again, including after `claims rebuild`.
- **S27** WHEN a pair is within one namespace or includes a `code:Bot`, THEN it's never a candidate.

**Reading identities**
- **S28** WHEN `about me` runs with linked GitHub and Linear accounts, THEN the result lists the Person, each linked account with its reason, and the accounts' claims, each citing its own account, with equivalent claims grouped and counted once toward `limit`.
- **S29** WHEN `about` names an account linked to a Person, THEN the result includes `rastro:held_by` that Person.
- **S30** WHEN a scoped read can't see any of a Person's linked accounts, THEN `about` on that Person equals the unknown-entity answer; `about me` in any scope shows only the visible accounts and claims.
- **S31** WHEN the random scope-set oracle runs with intents and automatic links, THEN each scoped answer, including links visible only where both ends are, equals a per-scope rebuild.
- **S32** WHEN `about` names a well-formed identifier that resolves to nothing visible (unknown, or hidden-only), THEN the result says `resolved: false`; WHEN it resolves, THEN `resolved: true`. MCP gets the same field; the schema change is additive.
- **S33** WHEN `about me` would exceed the work budget, THEN it fails with the budget error instead of a partial answer.
- **S36** WHEN the owner undoes a link or a merge, or names a Person again, THEN the undone claim (or the older name) stays as `superseded` with its interval closed at that time, `about` shows it as historical, and the older name still resolves to the Person.
- **S37** WHEN an intent is issued, THEN its `issuer_context` lists the issuing surface's configured scopes, and `rastro identity history` shows them.

**Accounts**
- **S35** WHEN a link, owner or automatic, names an account side whose type no vocabulary declares an account (a `code:PullRequest`, a `code:Repository`), THEN it's refused (an owner link with an operation error naming the reason; an automatic link is never made), while `code:Account` and `tracker:Account` link as before.
- **S38** WHEN a Linear sync reads an app (integration) user, THEN it's typed `tracker:Bot`, never `tracker:Account`, and a link to it is refused.
- **S39** WHEN `rastro source add --connector linear` runs while a Linear source exists, THEN it's refused with an error naming the existing source, and nothing is saved.

**Live**
- **S34** WHEN the ignored live test runs on a copy of the owner's database after the owner synced `svc-platform` and Linear and ran `rastro identity me`, THEN `about me` lists the owner's GitHub and Linear accounts, and `rastro identity candidates` returns without error.

## Out of scope
- MCP write operations for identity (D3).
- Stored hypotheses, the question queue, Asks on lapse (slices 5–6).
- Claim confirm, retract and fix intents; lifecycle binding; the `confirmed` status (slice 7; D6, D10).
- Address-book or Rule-minted Persons (knowledge §36 allows them; later).
- Linear comments and issue-to-PR links (slice 5).
- `source rename/remove` and extractor unbind (open question #9).
- Forgetting intents (slice 14; D22) and export (slice 13).

## Risks
- **Security:**
  - The Linear API key and GitHub token are read at sync only; S22 checks the key never lands in the database or config.
  - Scopes: Persons, links and candidates come from visible justifications only (S30, S31); the issuer must see the target (S9).
  - `/user/emails` needs the `user:email` token scope; without it, coverage says so (S13). The owner may need `gh auth refresh -s user:email`.
  - Work data (Heritage Linear, svc-platform) lives only in the local database under scope `work`.
- **Performance:**
  - `about me` spans every linked account's claims; `limit` and the existing work budget bound it (S33). The scoped benchmark gate is rerun with intents.
  - Candidates compare accounts across namespaces on read; expected sizes are tens of GitHub accounts and about a hundred Linear users.
  - The GitHub connector fetches each author's profile name once per sync.
- **Migration:** one new migration adds the `intent` table; the evidence-derived projection gets new claim kinds. Run `claims rebuild` after upgrading. Existing contracts change only additively (`resolved`, the Person view).
- **Rollback:** revert the PRs. The `intent` table is precious: an older binary ignores it, and it isn't dropped. The projection is disposable, so `claims rebuild` on the old binary restores it.

## Seams
- **`rastro` CLI with a temp `RASTRO_HOME`**, the scripted connector and extractor, and `--json` against the v2 schema (`crates/rastro-cli/tests/`): S1–S8, S10, S14–S19, S23–S29, S32, S33.
- **Storage-level tests** (`crates/rastro-storage-sqlite/tests/`): S9, S11, S16–S18, intent table constraints.
- **Connector tests against a local mock API** (`crates/rastro-connector-github/tests/`, `crates/rastro-connector-linear/tests/`): S12, S13, S20–S22.
- **`rastro mcp --scope` with the scripted client** (`crates/rastro-cli/tests/mcp.rs`): S30–S32.
- **Ignored test on a copy of the real database** (`crates/rastro-cli/tests/live.rs`): S34.

## Tickets
| # | What it delivers | Satisfies | Blocked by | Paths | Owner | Status |
|---|---|---|---|---|---|---|
| T1 | ADR 0010 (identity and owner intents: the D7 table, D8 resolution, D9 exemption, D10 status, D12 visibility, D13–D14 link formulas, D16 Person ids, the mixed-status group display). Decision entries in knowledge (amend §12, §46; §36 clarifications), vocabulary (`rastro:Person`, `held_by`, `same_as`, `name`, `email`, `tracker`), surfaces (`rastro identity`, `resolved`, the Person view), plugins (manifest-declared options). Glossary: Intent's identity kinds. Close open question #10. Roadmap slice 4 wording and status, with the README diagram | none (docs) | none | docs/adr/, docs/decisions/, docs/GLOSSARY.md, docs/open-questions.md, docs/roadmap.md, README.md | agent | COMPLETE (#87) |
| T2 | Manifest-declared source options and generic `source add` (D21) | S1, S2 | T1 | crates/rastro-core/src/lib.rs, crates/rastro-core/src/host/, crates/rastro-plugin-sdk/, crates/rastro-operations/, crates/rastro-cli/, crates/rastro-connector-github/rastro-plugin.toml, crates/rastro-test-connector/, schema/, Cargo.lock, docs/decisions/plugins.md, docs/decisions/surfaces.md, crates/rastro-core/src/runs/mod.rs | agent | COMPLETE (#88; converge, review, CI passed) |
| T3 | The `intent` table, `rastro identity` write commands and history, intent-backed Person, name, link and merge claims, rebuild replay, cache invalidation (D5–D11, D15, D16) | S3, S4, S5, S6, S7, S8, S9, S10, S11 | T1 | crates/rastro-storage-sqlite/, crates/rastro-core/src/, crates/rastro-operations/, crates/rastro-cli/, schema/ | agent | COMPLETE (#89; converge, review, CI passed) |
| T4 | Linear connector and extractor with the `tracker` vocabulary; API key handling; token-owner observation (D19, D22) | S20, S21, S22 | T2 | crates/rastro-connector-linear/, crates/rastro-extractor-linear/, Cargo.toml, Cargo.lock, crates/rastro-cli/tests/ | agent | COMPLETE (#91; converge, review, CI passed) |
| T5 | Reading identities: `about(person)`, `about me`, `held_by` on accounts, scoped visibility of Persons and links, `resolved`, superseded undone links and older names, recorded issuer scopes (D12, D18, D20, D26, D28) | S28, S29, S30, S31, S32, S33, S36, S37 | T3 | crates/rastro-storage-sqlite/, crates/rastro-core/src/, crates/rastro-operations/, crates/rastro-cli/, schema/ | agent | TO-DO |
| T6 | Automatic links: GitHub token owner, verified emails and profile names; the GitHub token owner recorded as `viewer`; derived email and token-owner links with intervals and cannot-link veto; guest emails unverified (D13–D15, D32, D34) | S12, S13, S14, S15, S16, S16a, S17, S18, S19 | T3, T4, T9 | crates/rastro-connector-github/, crates/rastro-extractor-github/, crates/rastro-core/src/, crates/rastro-storage-sqlite/, crates/rastro-cli/tests/, crates/rastro-test-connector/, crates/rastro-test-extractor/ | agent | TO-DO |
| T7 | Fuzzy candidates and answers (D4, D17) | S23, S24, S25, S26, S27 | T5, T6 | crates/rastro-core/src/, crates/rastro-storage-sqlite/, crates/rastro-operations/, crates/rastro-cli/, schema/ | agent | TO-DO |
| T9 | `rastro:Account` supertype: vocabularies declare their account types; core and storage refuse a link whose account side isn't one; Linear app users as `tracker:Bot`; one Linear source (D25, D31, D33) | S35, S38, S39 | T3, T4 | crates/rastro-core/src/, crates/rastro-storage-sqlite/, crates/rastro-plugin-sdk/, crates/rastro-extractor-github/, crates/rastro-extractor-linear/, crates/rastro-connector-linear/, crates/rastro-operations/, crates/rastro-test-extractor/, crates/rastro-cli/tests/ | agent | TO-DO |
| T8 | Owner run: `gh auth refresh -s user:email`; set `LINEAR_API_KEY`; add `svc-platform` (GitHub, `work`) and Linear (`work`) sources; `rastro identity me`; sync; review candidates; run the live test on a copy; sign off | S34 | T5, T6, T7 | crates/rastro-cli/tests/live.rs | fbisca | TO-DO |
