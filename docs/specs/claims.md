# Claims
Paths: crates/rastro-extractor-github/, crates/rastro-test-extractor/, crates/rastro-plugin-sdk/, crates/rastro-core/src/claims.rs, crates/rastro-core/src/knowledge.rs, crates/rastro-core/src/entity_form.rs, crates/rastro-core/src/host/, crates/rastro-core/src/runs/, crates/rastro-storage-sqlite/src/knowledge.rs, crates/rastro-storage-sqlite/src/lifecycle.rs, crates/rastro-storage-sqlite/src/projection_temp.sql, crates/rastro-storage-sqlite/migrations/, crates/rastro-storage-sqlite/tests/alias_lookup.rs, crates/rastro-storage-sqlite/tests/lifecycle.rs, crates/rastro-storage-sqlite/tests/location_context.rs, crates/rastro-storage-sqlite/tests/hub_scaling.rs, crates/rastro-storage-sqlite/tests/migrations.rs, crates/rastro-cli/tests/claims.rs, crates/rastro-cli/tests/about_why.rs, crates/rastro-cli/tests/live.rs

## Requirements

### Requirement: A bound extractor runs over its source's pending evidence
The core SHALL run a source's bound extractor over that (source, scope)'s pending evidence after a sync accepts it, and its claims SHALL be readable through `about` and `why`.

#### Scenario: the sync's evidence becomes readable claims
- WHEN a sync accepts evidence for a source bound to an extractor, THEN the extractor runs over that (source, scope)'s pending evidence, its claims can be read through `about` and `why`, and the extractor run is logged as its own run with plugin id, version and counts

#### Scenario: coverage is the same as without the extractor
- WHEN a sync whose source is bound to an extractor completes, THEN the source's coverage is the same as without the extractor

### Requirement: A support that fails its check rejects the claim
The core SHALL record nothing for a claim whose support is untrusted evidence, evidence not delivered to that run, a span outside its evidence, or a rule not declared deterministic, and SHALL count it rejected.

#### Scenario: one failing support records nothing
- WHEN an extractor submits a claim whose support is untrusted evidence, evidence not delivered to that run, a span outside its evidence, or a rule not declared deterministic, THEN the core records nothing for that claim and counts it rejected

### Requirement: A claim outside the vocabulary is rejected
The core SHALL reject and count a submitted claim whose predicate isn't in the vocabulary, or whose subject or object breaks the predicate's domain or range.

#### Scenario: the predicate or an endpoint is outside the vocabulary
- WHEN a submitted claim's predicate isn't in the vocabulary, or its subject or object breaks the predicate's domain or range, THEN it's rejected and counted

### Requirement: One failing span rejects the whole claim
The core SHALL reject and count the whole claim when one span of its support bundle fails a check, even though its other spans pass.

#### Scenario: a bundle is accepted or refused as one claim
- WHEN one span of a claim's support bundle fails a check, THEN the whole claim is rejected and counted, even though its other spans pass

### Requirement: A type must come from the submission that uses it
The core SHALL reject and count a claim whose endpoint's type isn't given by a `rastro:type` claim in the same submission, and SHALL keep both types visible as a conflict when a submitted `rastro:type` disagrees with a type live anywhere on the entity and neither is a `supertype` of the other.

#### Scenario: an endpoint typed only by an earlier submission is refused
- WHEN a submitted claim has an endpoint whose type isn't given by a `rastro:type` claim in the same submission, THEN it's rejected and counted, even if an earlier submission typed that entity

#### Scenario: disagreeing types at different locations conflict
- WHEN a submission's `rastro:type` disagrees with a type live anywhere on the entity (any location, any submission) and neither is a `supertype` of the other, THEN both stay visible as a conflict

### Requirement: A pair without its extractor reads not installed
The sync SHALL succeed and `about` SHALL report extraction state `not_installed` for a pair whose bound extractor isn't installed; an installed extractor that isn't bound to a source SHALL never run, and `claims rebuild` SHALL carry that pair's output over instead of dropping it.

#### Scenario: a missing extractor leaves the sync and the read whole
- WHEN a source's bound extractor isn't installed, THEN the sync succeeds and `about` reports extraction state `not_installed` for that pair

#### Scenario: an unbound extractor never runs
- WHEN an extractor is installed but not bound to the source, THEN it never runs

#### Scenario: a rebuild carries the pair's output over
- WHEN `claims rebuild` runs over a pair whose extractor isn't installed, THEN that pair's output is carried over instead of dropped

### Requirement: An existing GitHub source is bound to its extractor
Loading an existing GitHub source SHALL bind it to `rastro-extractor-github` without the owner editing config.

#### Scenario: the binding needs no config edit
- WHEN an existing GitHub source is loaded, THEN it is bound to `rastro-extractor-github` without the owner editing config

### Requirement: Two sources that see the same state share one claim
Two sources observing the same PR in the same state SHALL produce one PR entity and one `code:state` claim with two supports.

#### Scenario: the same state observed twice is one claim
- WHEN two sources observe the same PR with the same state, THEN there is one PR entity and one `code:state` claim with two supports

### Requirement: A second support keeps the id and moves the revision
A claim that gains a second support SHALL keep its id and change its revision.

#### Scenario: corroboration keeps the claim and moves its revision
- WHEN a second support arrives for an existing claim, THEN the claim id stays the same and the revision changes

### Requirement: A claim's id and revision survive replay
Replaying the same evidence into a fresh database, or changing the bound extractor's version without changing a claim's visible state, SHALL leave the claim's id and its `rastro/revision/v2` revision unchanged.

#### Scenario: a replay keeps ids and revisions
- WHEN the same evidence is replayed into a fresh database with other local evidence ids, or the bound extractor's version changes without changing a claim's visible state, THEN the claim's id and its `rastro/revision/v2` revision are unchanged

### Requirement: A state that returns keeps its id across supersession
A PR state observed open, then closed, then open SHALL keep the `open` claim's id, mark it `superseded` and then `observed` again with a new revision, show both known intervals in `why`, and leave the `closed` claim `superseded` with `superseded_by` pointing at the `open` claim.

#### Scenario: a state that comes back reuses its claim
- WHEN a PR's state goes open → closed → open, THEN the `open` claim keeps its id, is `superseded` and then `observed` again with a new revision, `why` shows both known intervals, and the `closed` claim is `superseded` with `superseded_by` pointing at the `open` claim

### Requirement: Recurrences at one captured time stay separate and ordered
States observed at the same `captured_at` SHALL read as separate occurrences ordered by event key, each allowed to be zero-length.

#### Scenario: three states share one captured time
- WHEN a PR's state is observed open, then closed, then open, all at the same `captured_at`, THEN `why` shows two separate occurrences of the `open` claim and one of the `closed` claim, ordered by event key and allowed to be zero-length, and the `open` claim is `observed`

### Requirement: A missing field supersedes nothing
A later observation that lacks a field, has it null, or isn't returned at all SHALL supersede no claim.

#### Scenario: an absent field leaves the claim alone
- WHEN a later observation lacks a field, has it null, or the item wasn't returned, THEN no claim is superseded

### Requirement: The newer comparable observation wins its slot
The newer and comparable of two sources' disagreeing observations of the same PR state SHALL be current and the older `superseded`, a tie or an incomparable pair SHALL leave both `observed` with `conflicts_with`, and dominance and supersession SHALL hold per (location, field) with only `rastro:type` conflicts entity-wide.

#### Scenario: the newer comparable observation is current
- WHEN two sources' current observations of the same PR state disagree and one is newer and comparable, THEN the newer is current and the older is `superseded`

#### Scenario: a tie or an incomparable pair conflicts
- WHEN two sources' observations of the same PR state tie or can't be compared, THEN both stay `observed` with `conflicts_with`

#### Scenario: dominance is per location and field
- WHEN claims disagree, THEN dominance and supersession hold per (location, field), and only `rastro:type` conflicts are entity-wide

### Requirement: One span can justify two claims
A rule that derives two claims from one span SHALL store both, each with its own justification, and neither SHALL replace the other.

#### Scenario: one URL yields a repository and a kind
- WHEN one rule derives two claims from the same span (a repository and a kind from one URL), THEN both are stored, each with its own justification, and neither replaces the other

### Requirement: A claim's known interval comes from its evidence
A derived claim's known interval SHALL start at the establishing evidence's `captured_at` and end at the replacing evidence's `captured_at`, and its valid time SHALL read `unknown`.

#### Scenario: known time follows the evidence, valid time is unknown
- WHEN a claim is derived, THEN its known interval starts at the establishing evidence's `captured_at`, ends at the replacing evidence's `captured_at`, and its valid time reads `unknown`

### Requirement: A renamed account keeps its entity and both logins
An account whose login changes SHALL stay one entity, typed `code:Account` by a `rastro:type` claim, with both logins as `rastro:alias` claims whose known intervals show when each held; the old login SHALL be `superseded` and still resolve, `/user/type` `Bot` SHALL type it `code:Bot`, no entity SHALL be typed `Person`, and no type SHALL be stored as a column or updated in place.

#### Scenario: both logins stay as aliases of one account
- WHEN an account's login changes between observations, THEN it stays the same entity, typed `code:Account` by a `rastro:type` claim, with both logins as `rastro:alias` claims whose known intervals show when each held, the old one `superseded` yet still resolving

#### Scenario: a bot is typed a bot and a type is never a column
- WHEN `/user/type` is `Bot`, THEN its `rastro:type` claim is `code:Bot`; no entity is typed `Person`, and no type is stored as a column or updated in place

### Requirement: Instruction-like text yields no claim or label
No claim or display name SHALL come from instruction-like text in a title or body.

#### Scenario: injected instructions are inert
- WHEN a title or body holds instruction-like text, THEN no claim or display name comes from it

### Requirement: The GitHub extractor emits cited code claims
`rastro-extractor-github` SHALL emit `code:state`, `code:author` and `code:in_repository` claims with their `rastro:type` claims from the GitHub fixture, each cited at the right field and span, with author and alias claims justified by one bundle of all the spans they need, and the core SHALL load its `code` vocabulary, which has every required definition field and a `cardinality` on every predicate.

#### Scenario: the fixture's claims are cited at their spans
- WHEN `rastro-extractor-github` processes the GitHub fixture, THEN it emits `code:state`, `code:author` and `code:in_repository` claims (with their `rastro:type` claims), each cited at the right field and span, with author and alias claims justified by one bundle of all the spans they need

#### Scenario: the code vocabulary is complete
- WHEN the core loads the `code` vocabulary, THEN it has every required definition field and a `cardinality` on every predicate

### Requirement: The core refuses a vocabulary it cannot hold
The core SHALL refuse to load a vocabulary file with an unknown field, a predicate without `cardinality`, or a name under its `rastro:` prefix, with an error naming the problem, and SHALL accept no claim under it.

#### Scenario: a bad vocabulary names its problem and loads nothing
- WHEN a vocabulary file has an unknown field, a predicate without `cardinality`, or a name under the core's `rastro:` prefix, THEN the core refuses to load it with an error naming the problem, and no claim is accepted under it

### Requirement: A changed field arrives with its companion fields
When a record's changed field reaches the extractor, the core SHALL also deliver the companion fields of the same source, scope and location with evidence no newer than the batch's high-water mark, and a rebuild SHALL deliver the same set.

#### Scenario: the companions come along
- WHEN a record's changed field reaches the extractor, THEN the core also delivers the companion fields of the same source, scope and location with evidence no newer than the batch's high-water mark

#### Scenario: a rebuild delivers the same set
- WHEN a pair is rebuilt, THEN it delivers the same companion fields as the incremental run

### Requirement: A rebuild equals the incremental projection
`rastro claims rebuild` after a series of incremental syncs SHALL produce a projection equal to the incremental one — entity and claim ids, revisions, statuses, supports, intervals — when every pair replays, for a GitHub source and a scripted source alike.

#### Scenario: a GitHub and a scripted source rebuild equal
- WHEN `rastro claims rebuild` runs after a series of incremental syncs and every pair replays, THEN the projection equals the incremental one for a GitHub source and for a scripted (non-GitHub) source alike

#### Scenario: a pair that cannot replay follows the carry-over rules
- WHEN a pair can't replay, THEN its output is carried over and the command's result and exit code follow the carry-over rules

### Requirement: A new extractor version replays before retiring the old claims
When the bound extractor's version changes, the next sync SHALL replay its pairs, reads SHALL serve the old version's claims until that replay completes, and the new version's justifications SHALL then replace the old ones in one transaction.

#### Scenario: old claims serve until the replay retires them
- WHEN the bound extractor's version changes, THEN the next sync replays its pairs, reads serve the old version's claims until that replay completes, and then the new version's justifications replace the old ones in one transaction

### Requirement: Every publication records its receipt
A rebuild or a sync's extraction that publishes the projection SHALL record one `projection_publication` row with the extractor and vocabulary versions and the evidence high-water mark, that row SHALL not be updated or deleted, and the projection tables SHALL have fresh `ANALYZE` statistics.

#### Scenario: a publication writes one immutable receipt
- WHEN a rebuild or a sync's extraction publishes the projection, THEN one `projection_publication` row records the extractor and vocabulary versions and the evidence high-water mark, and that row can't be updated or deleted

#### Scenario: the published tables carry fresh statistics
- WHEN the projection is published, THEN the projection tables have fresh `ANALYZE` statistics

### Requirement: Evidence a failed sync kept is extracted next time
A sync that fails after accepting some evidence SHALL have that evidence extracted by the next writing sync, even if it accepts nothing new.

#### Scenario: the next writing sync extracts what the failed one kept
- WHEN a sync fails after accepting some evidence, THEN the next writing sync extracts that evidence, even if it accepts nothing new

### Requirement: A failed extraction fails the sync and keeps reads whole
When the extractor crashes or fails during a sync, `sync` SHALL exit non-zero, the source's coverage SHALL stay as read, extraction state SHALL read `failed`, reads SHALL serve the last good projection, and the next sync SHALL resume from the checkpoint.

#### Scenario: a crash fails the sync and leaves the read side whole
- WHEN the extractor crashes or fails during a sync, THEN `sync` exits non-zero, the source's coverage stays as read, extraction state reads `failed`, reads serve the last good projection, and the next sync resumes from the checkpoint

### Requirement: A failed version replay keeps the old version's claims
A replay for a new extractor version that fails SHALL leave the old version's claims readable and unchanged, and extraction state SHALL read `failed`.

#### Scenario: a failed replay changes nothing readable
- WHEN the replay for a new extractor version fails, THEN the old version's claims stay readable and unchanged, and extraction state reads `failed`

### Requirement: An unfinished extractor run is marked interrupted
The next writing open SHALL mark an extractor run left unfinished as interrupted, without creating a source coverage window.

#### Scenario: an abandoned run is interrupted, not counted
- WHEN an extractor run was left unfinished, THEN the next writing open marks it interrupted without creating a source coverage window

### Requirement: A failed rebuild leaves the previous projection
A rebuild that fails partway, or an extractor that starts and then fails, SHALL leave the previous projection readable and unchanged, and a reader running during a rebuild SHALL see either the old or the new projection, never a mix.

#### Scenario: a failed rebuild keeps the previous projection
- WHEN `rastro claims rebuild` fails partway, or an extractor starts and then fails, THEN the previous projection stays readable and unchanged

#### Scenario: a reader during a rebuild sees one generation
- WHEN a reader runs during a rebuild, THEN it sees either the old or the new projection, never a mix

### Requirement: About resolves an entity's current claims
`about` SHALL return the current claims where an entity is subject or object — each with claim id, status, revision, predicate, subject and object aliases and a support count — when given an entity id, a canonical alias or an exact canonical URL, and SHALL return an operation error naming the accepted forms for any other form.

#### Scenario: the accepted forms answer the entity's claims
- WHEN `about` is called with an entity id, a canonical alias or an exact canonical URL, THEN it returns the current claims where that entity is subject or object, each with claim id, status, revision, predicate, subject and object aliases and a support count

#### Scenario: any other form is an operation error
- WHEN `about` is called with any other form, THEN it returns an operation error naming the accepted forms

### Requirement: A login resolves through its alias claim
`about` called with `github:login/<login>` for a login recorded as a `rastro:alias` claim of one account, whose status is anything but `hypothesis` or `retracted` (today `observed` or `superseded`), SHALL return that account's claims exactly as for `github:account/<id>`, a namespace or kind that doesn't match the stored alias SHALL return the empty result with coverage, and a login several entities hold SHALL answer as the ambiguous-alias requirement says.

#### Scenario: the login answers as its account does
- WHEN `about` is called with `github:login/<login>` for a login recorded as a `rastro:alias` claim of one account, whose status is anything but `hypothesis` or `retracted` (today `observed` or `superseded`), THEN it returns that account's claims exactly as for `github:account/<id>`

#### Scenario: several entities holding the login answer as ambiguous
- WHEN several entities hold the login, THEN `about` on it answers as the ambiguous-alias requirement says, naming every candidate

#### Scenario: a mismatched namespace or kind is empty with coverage
- WHEN the namespace or kind doesn't match the stored alias, THEN it returns the empty result with coverage

### Requirement: About filters, orders and bounds
Given `since`, `about` SHALL return only claims whose known interval started at or after it, newest known time first and then by claim id; `limit` SHALL default to 20, accept 1–50, and `truncated` SHALL say more exist; `history: true` SHALL include superseded claims.

#### Scenario: since, limit and history bound the answer
- WHEN `since` is given, THEN only claims whose known interval started at or after it are returned, newest known time first, then claim id; `limit` defaults to 20, accepts 1–50, and `truncated` says more exist; with `history: true`, superseded claims are included

### Requirement: An unknown entity reads as empty with coverage
`about` and `why` SHALL return the same empty result with coverage for an entity that is unknown and for one visible only through hidden scopes.

#### Scenario: unknown and hidden-only answer alike
- WHEN `about` names an unknown entity or one visible only through hidden scopes, THEN both return the same empty result with coverage

### Requirement: Reads carry coverage and extraction state per pair
`about` and `why` SHALL include coverage for every visible (source, scope) pair and extraction state per pair and extractor version, and a pair with evidence pending extraction SHALL read `behind`.

#### Scenario: every visible pair is accounted for
- WHEN `about` or `why` answers, THEN it includes coverage for every visible (source, scope) pair and extraction state per pair and extractor version

#### Scenario: pending evidence reads behind
- WHEN a pair has evidence pending extraction, THEN that pair reads `behind`

### Requirement: Why explains a claim and bounds its supports
`why` SHALL return the claim with status, revision, known intervals, valid time, conflict and supersession links and the extractor, rule and vocabulary versions, plus at most `limit` supports with `truncated`.

#### Scenario: the claim and its versions are explained
- WHEN `why` is called on a claim, THEN it returns the claim with status, revision, known intervals, valid time, conflict and supersession links and the extractor, rule and vocabulary versions, plus at most `limit` supports with `truncated`

#### Scenario: each support carries its citation and excerpt
- WHEN `why` lists a support, THEN it carries evidence id, source, scope, location, field, span, `captured_at`, `source_updated_at`, trust class and an excerpt that starts and ends on UTF-8 boundaries with clipping flags, the full quote appears only with `full`, and the support names its justification kind, extractor and version and lists every span of its bundle

### Requirement: An unknown claim is an operation error
`why` SHALL return the same operation error for an unknown claim id and for one visible only through hidden scopes, as a result with `isError: true` on MCP.

#### Scenario: unknown and hidden-only claims answer alike
- WHEN `why` gets an unknown or hidden-only claim id, THEN it returns the same operation error (`isError: true` on MCP)

### Requirement: The owner's claims and receipts match GitHub
After `rastro sync --reread` on `kalios` and `kalios-api`, `about` on their GitHub account alias and `why` on a returned claim in Claude Code SHALL match GitHub's claims and receipts.

#### Scenario: the owner reads his real accounts
- WHEN the owner runs `rastro sync --reread` on `kalios` and `kalios-api` and then asks `about` for their GitHub account alias and `why` on one returned claim in Claude Code, THEN the claims and receipts match GitHub

### Requirement: An old login still resolves to its account
`about github:login/<old>` SHALL return that account's claims, as `about github:account/<id>` does, after a re-read superseded the old login's alias claim.

#### Scenario: a superseded login still resolves
- WHEN an account's login changed and a re-read superseded the old login's alias claim, THEN `about github:login/<old>` returns that account's claims, as `about github:account/<id>` does

### Requirement: A login held twice names both candidates
`about` on a login that is an alias of two accounts SHALL return the ambiguous-alias operation error listing both candidates, each marked `current` or `historical`; a login one entity holds SHALL resolve directly.

#### Scenario: an ambiguous login lists its candidates
- WHEN one login is an alias of two accounts (current on one, historical on the other), THEN `about` on it returns the ambiguous-alias operation error listing both candidates, each marked `current` (observed or confirmed) or `historical` (superseded)

#### Scenario: one entity holding the login resolves directly
- WHEN one entity holds the login, even current and historical both, THEN it resolves directly

### Requirement: A resolution through an old alias lists current aliases
`about` resolving through an old login SHALL list only current aliases as its subject aliases, and `why` on the old alias claim SHALL show its known intervals.

#### Scenario: old and current aliases read differently
- WHEN `about` resolves through an old login, THEN its subject aliases list only current aliases, and `why` on the old alias claim shows its known intervals

### Requirement: A hypothesis or retracted alias is no lookup candidate
An alias claim with status `hypothesis` or `retracted` SHALL not be a lookup candidate.

#### Scenario: only live aliases resolve
- WHEN an alias claim with status `hypothesis` or `retracted` matches, THEN it isn't a lookup candidate

### Requirement: A pair without its extractor is carried over
`claims rebuild` SHALL carry a pair's justifications, spans, claims, entities and checkpoint over when the pair's bound extractor isn't installed, recompute every touched component's statuses, occurrences and links, rebuild the other pairs, exit 0, and list the pair with state `not_installed`.

#### Scenario: the carried-over pair is rebuilt around
- WHEN `claims rebuild` runs and a pair's bound extractor isn't installed, THEN that pair's justifications, spans, claims, entities and checkpoint are carried over, every touched component's statuses, occurrences and links are recomputed, the other pairs are rebuilt, the command exits 0, and its result lists the pair with state `not_installed`

### Requirement: A forbidden or failing pair is carried over and fails the rebuild
When a pair's extractor fails its check, or the pair's scope isn't in the extractor manifest's `permissions.scopes`, `claims rebuild` SHALL carry its output over, read the pair `failed` with its reason (`forbidden` for the scope case), and exit non-zero.

#### Scenario: a failing or forbidden pair carries over and fails the command
- WHEN a pair's extractor fails its check, or the pair's scope isn't in the extractor manifest's `permissions.scopes`, THEN its output is carried over, the pair reads `failed` with its reason (`forbidden` for the scope case), and the command exits non-zero

### Requirement: An extractor that fails mid-rebuild aborts it
An extractor that starts during a rebuild and then fails SHALL abort the rebuild, leaving the previous projection readable and unchanged.

#### Scenario: a failure after the start ends the rebuild
- WHEN an extractor starts during a rebuild and then fails, THEN the rebuild aborts and the previous projection stays readable and unchanged

### Requirement: A claim carried and rebuilt reads as incremental
A claim justified by both a carried-over pair and a rebuilt pair SHALL have the status, occurrences, links and revision of the incremental projection.

#### Scenario: a shared justification reads as incremental
- WHEN a claim is justified by both a carried-over pair and a rebuilt pair, THEN its status, occurrences, links and revision equal those of the incremental projection

### Requirement: A rebuild lists every pair
`claims rebuild` SHALL return a `pairs` list with source, scope, run id, extractor, version, state, reason and counts, the CLI SHALL print a line per carried-over pair, and the schema change SHALL be additive within v2.

#### Scenario: the command reports each pair
- WHEN `claims rebuild` completes, THEN its result has a `pairs` list with source, scope, run id, extractor, version, state, reason and counts, the CLI prints a line per carried-over pair, and the schema change is additive within v2 (the drift test accepts it as recorded)

### Requirement: A rebuild keeps every claim of the owner's database
`claims rebuild` on a copy of the owner's real database with the extractor plugin hidden SHALL keep all of the copy's claims.

#### Scenario: the owner's copy loses nothing
- WHEN `claims rebuild` runs on a copy of the owner's real database with the extractor plugin hidden, THEN all of the copy's claims remain (an ignored test, given the copy's path)

### Requirement: Unrelated types at different locations conflict
An entity with live `rastro:type` claims of unrelated types at different locations SHALL carry `conflicts_with` on both, neither superseding the other; only `rastro:type` conflicts SHALL be entity-wide, and aliases and other predicates SHALL stay per (location, field).

#### Scenario: unrelated types conflict both ways
- WHEN an entity has live `rastro:type` claims of unrelated types at different locations, THEN both carry `conflicts_with` and neither supersedes the other

#### Scenario: only type conflicts are entity-wide
- WHEN claims disagree, THEN only `rastro:type` conflicts are entity-wide, and aliases and other predicates stay per (location, field)

### Requirement: A type and its supertype never conflict
A type and one of its supertypes at different locations SHALL not conflict, and `about` and `why` SHALL display the most specific type.

#### Scenario: related types coexist
- WHEN the types at different locations are a type and one of its supertypes, THEN there's no conflict, and `about` and `why` display the most specific type

### Requirement: A conflicted entity displays one type
When an entity's types conflict, `about` and `why` SHALL display one type: live types (observed or confirmed) grouped into compatible families, each headed by a most specific type (one no other live type descends from) and holding the live ancestors it carries, a family ranked by its newest member's known time with ties going to the lowest claim id of the family's head, the newest family's head type displayed, and branching subtypes of one supertype counting as separate families that each count the supertype's time (Draft on day 1 under PullRequest on day 3, against Repository on day 2, displays Draft); a scoped read SHALL compute it from visible justifications only.

#### Scenario: the newest family's head is the displayed type
- WHEN an entity's types conflict, THEN `about` and `why` display one type: live types (observed or confirmed) grouped into compatible families, each headed by a most specific type (one no other live type descends from) and holding the live ancestors it carries, the newest family's head type displayed, ties going to the lowest claim id of the family's head, branching subtypes of one supertype counting as separate families that each count the supertype's time (Draft on day 1 under PullRequest on day 3, against Repository on day 2, displays Draft)

#### Scenario: a scoped read ranks only what it can see
- WHEN a scoped read displays a conflicted entity's type, THEN it computes the families and the displayed type from visible justifications only

### Requirement: A type changed at one location supersedes the old one
A change of the same location's type SHALL have the new type claim supersede the old one.

#### Scenario: one location's type moving on
- WHEN the same location's type changes (Bot to User), THEN the new type claim supersedes the old one

### Requirement: A new observation replays only its slots
A new item observation for an entity typed at many locations SHALL replay only the affected (location, field) slots, keep the hub scaling bound, and equal the incremental projection under a rebuild.

#### Scenario: only the affected slots replay
- WHEN a new item observation arrives for an entity typed at many locations, THEN only the affected (location, field) slots replay, the hub scaling bound still holds, and a rebuild equals the incremental projection

### Requirement: A malformed entity form is an operation error
`about` given `foo:bar`, `github:FBisca`, `github:login/`, `:x/y`, an `e_` id with uppercase hex, or a form with whitespace in its namespace or kind or at either end of its value SHALL return an operation error naming the three forms; on MCP it SHALL be a result with `isError: true`, not a JSON-RPC error.

#### Scenario: a malformed form names the three accepted forms
- WHEN `about` gets `foo:bar`, `github:FBisca`, `github:login/`, `:x/y`, an `e_` id with uppercase hex, or a form with whitespace in its namespace or kind or at either end of its value, THEN it returns an operation error naming the three forms; on MCP it's a result with `isError: true`, not a JSON-RPC error

### Requirement: A well-formed form that matches nothing is empty
`about` given a well-formed form that matches nothing SHALL return the empty result with coverage.

#### Scenario: no match is not an error
- WHEN `about` gets a well-formed form that matches nothing (`gitlab:login/x`, `github:login/FBisca/x`, `github:login/a b`), THEN it returns the empty result with coverage

### Requirement: Every real identifier is accepted
`about` SHALL accept a canonical URL with a fragment, `github:account/<id>` and `github:login/vercel[bot]`.

#### Scenario: the awkward real identifiers all resolve
- WHEN `about` gets a canonical URL with a fragment, `github:account/<id>` or `github:login/vercel[bot]`, THEN each is accepted

### Requirement: An identifier outside the entity forms is rejected
The core SHALL reject and count a submitted claim whose subject or object identifier is outside the accepted forms.

#### Scenario: a bad identifier records nothing
- WHEN an extractor submits a claim whose subject or object identifier is outside the accepted forms, THEN it's rejected and counted

### Requirement: Alias kinds and values are checked
Loading a vocabulary whose alias kind contains `/`, `:`, or whitespace SHALL be refused; a submitted alias claim whose value is empty, whitespace-only, or has whitespace at either end SHALL be rejected and counted; one with inner whitespace SHALL be accepted and `about` SHALL resolve it.

#### Scenario: alias kinds and values are held to their forms
- WHEN a vocabulary declares an alias kind containing `/`, `:` or whitespace, THEN loading it is refused; WHEN a submitted alias claim's value is empty, whitespace-only, or has whitespace at either end, THEN it's rejected and counted; WHEN it has inner whitespace (`Felippe Bisca`), THEN it's accepted and `about` resolves it

### Requirement: A rebuild reproduces every intent-backed claim
`claims rebuild`, including with an extractor missing so a pair is carried over, SHALL bring every intent-backed Person, name, link and merge back identically, and the intent table SHALL be unchanged in rows and order.

#### Scenario: intents replay into the same claims
- WHEN `claims rebuild` runs, including with an extractor missing so a pair is carried over, THEN every intent-backed Person, name, link and merge reappears identically, and the intent table is unchanged (rows and order)

### Requirement: A GitHub issue or pull request side carries its title
`about` SHALL label a GitHub issue or pull request side whose location has `/title` evidence with the title text, `url` = the location, the evidence id, `field` `/title`, `captured_at`, `trust_class` untrusted content and `clipped` false.

#### Scenario: the side's label cites the title
- WHEN `about` returns a claim whose subject or object is a GitHub issue or PR with `/title` evidence at its location, THEN that side's label carries the title text, `url` = the location, the evidence id, `field` `/title`, `captured_at`, `trust_class` untrusted content and `clipped` false

### Requirement: Why carries the same labels as about
`why` SHALL give its subject and object the same labels as `about`.

#### Scenario: both reads label a side alike
- WHEN `why` explains a claim, THEN its subject and object carry the same labels as in `about`

### Requirement: A Linear issue and a Linear user are labelled from their own fields
A Linear issue's label SHALL come from `/title` with `url` null, and a Linear user's label from `/name`, not `/displayName`.

#### Scenario: the issue and the user each read their own field
- WHEN the entity is a Linear issue, or a Linear user, THEN a Linear issue's label comes from `/title` with `url` null; a Linear user's label comes from `/name`, not `/displayName`

### Requirement: A repository, comment, account or Person has no label
The label SHALL be null for a repository, a comment, a GitHub account and a Person.

#### Scenario: some entities carry no label
- WHEN the entity is a repository, a comment, a GitHub account or a Person, THEN its label is null

### Requirement: A long title is clipped on a character boundary
A label SHALL hold at most 200 whole characters of a longer title, and `clipped` SHALL be true.

#### Scenario: the cut lands on a character boundary
- WHEN a title is longer than 200 chars, with a multibyte char across the cut, THEN the label holds at most 200 whole chars and `clipped` is true

### Requirement: The latest observation's title is the label
With several title observations, the label SHALL be the one with the latest `captured_at`, ties broken by the higher evidence id.

#### Scenario: the newest title wins
- WHEN an entity has several title observations, THEN the label is the one with the latest `captured_at`, ties broken by the higher evidence id

### Requirement: A scope decides whose title is labelled
When a scope hides an entity's only title evidence, the label SHALL be null and the call SHALL still succeed; a visible copy in another scope SHALL be used.

#### Scenario: a hidden title leaves a null label
- WHEN `rastro mcp --scope <s>` hides the only title evidence of an entity, or a visible copy exists in another scope, THEN the label is null and the call still succeeds; with a visible copy in another scope, that copy is used

### Requirement: Labels change no claim, support or revision
Adding labels SHALL change no claim, support or revision, and `about "<a title's text>"` SHALL not resolve to the entity.

#### Scenario: labels are a read of the claims, not a write
- WHEN labels are added, THEN claims, supports and revisions are unchanged, and `about "<a title's text>"` doesn't resolve to the entity

### Requirement: The label carries the current state
`label.state` SHALL hold the value of an entity's current observed `code:state` or `tracker:state` claim, and SHALL be null when it has none.

#### Scenario: the label's state is the current one
- WHEN an entity has a current observed state claim (`code:state` or `tracker:state`), or has none, THEN `label.state` holds its value; with none, `label.state` is null

### Requirement: The CLI shows labels as the MCP result does
Without `--json` the CLI SHALL print each labelled entity's label after its id with control chars escaped; with `--json` the envelope SHALL equal the MCP result.

#### Scenario: human and machine output agree
- WHEN the CLI prints `about` or `why` without `--json`, or with `--json`, THEN each labelled entity shows its label after its id with control chars escaped; with `--json` the envelope equals the MCP result

### Requirement: The tool descriptions say label text is untrusted
Listing tools over MCP SHALL give `about` and `why` descriptions that say label text is untrusted data.

#### Scenario: an MCP client is told what a label is
- WHEN an MCP client lists tools, THEN the `about` and `why` descriptions say label text is untrusted data

### Requirement: A Linear state change supersedes its tracker state claim
Extracting a Linear issue's `/state/name` evidence SHALL give an observed `tracker:state` claim holding that name, superseded by the new one when the name changes.

#### Scenario: the state claim follows the name
- WHEN a Linear issue's `/state/name` evidence is extracted, and the name later changes, THEN an observed `tracker:state` claim holds that name; after the change the old claim is superseded and the new one is current

### Requirement: Stderr names the source and its read reason
Before a source syncs, stderr SHALL name the source and its reason: `first read from <window_start>` with no complete check, `incremental since <window_end>` otherwise, `re-read from <window_start>` with `--reread`.

#### Scenario: the read's reason precedes the read
- WHEN a source syncs, THEN before the read stderr names the source and its reason, `first read from <window_start>` with no complete check, `incremental since <window_end>` otherwise, `re-read from <window_start>` with `--reread`

### Requirement: Stderr shows the stages in order
A sync SHALL show the read, extract and project stages on stderr in order, and an extract line replaying stored evidence SHALL say so.

#### Scenario: read, extract and project in order
- WHEN a source syncs, and extract replays stored evidence because the extractor version changed, THEN stderr shows the read, extract and project stages in order; on a replay the extract line says so

### Requirement: A slow read shows running counts at most every two seconds
A source read lasting longer than 2 s SHALL show at least one running item count on stderr, and never more than one count line per 2 s.

#### Scenario: a slow read is counted, not chattered
- WHEN a read lasts longer than 2 s, THEN stderr shows at least one running item count, and never more than one count line per 2 s

### Requirement: The result line gives counts and the retry time
A source that finishes or ends `rate_limited` SHALL get a result line on stderr with its submitted, accepted and unchanged counts, giving the retry time when `rate_limited`.

#### Scenario: each source's result line
- WHEN a source finishes, or ends rate_limited, THEN stderr shows its result line with submitted, accepted and unchanged counts; when rate_limited the line gives the retry time

### Requirement: A broken stderr changes nothing
`sync --json` SHALL put only the one result envelope on stdout, and with stderr closed or broken `sync` SHALL complete with the same result and exit code.

#### Scenario: stdout stays one envelope
- WHEN `sync --json` runs, or stderr is closed or broken, THEN stdout holds only the one result envelope; with stderr closed or broken, sync completes with the same result and exit code

### Requirement: A renamed source is warned about before its full read
A configured source with no complete check, and a stored source no longer in config with the same connector and selector in any scope, SHALL produce a warning before the read naming both sources and listing it in the result's `warnings`, and the read SHALL still run from `window_start`.

#### Scenario: a renamed source is named before it reads
- WHEN a configured source has no complete check and a stored source no longer in config has the same connector and selector, in any scope, THEN before the read stderr warns, naming both sources, and the sync result's `warnings` lists it; the read still runs from `window_start`

### Requirement: No warning when a complete check exists
No warning SHALL be given and `warnings` SHALL be empty when the configured source already has a complete check or no stored-only source shares its connector and selector.

#### Scenario: a complete check or no twin means no warning
- WHEN the configured source already has a complete check, or no stored-only source shares its connector and selector, THEN no warning is given and `warnings` is empty

### Requirement: The owner preps a standup from about me
Once the slice has landed, the owner SHALL prep one morning's standup with `about me` over MCP, without opening Linear or GitHub.

#### Scenario: one read answers across sources
- WHEN the slice has landed, THEN the owner preps one morning's standup with `about me` over MCP without opening Linear or GitHub
