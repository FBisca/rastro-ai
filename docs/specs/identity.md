# Identity
Paths: crates/rastro-core/src/identity.rs, crates/rastro-core/src/identity/, crates/rastro-storage-sqlite/src/identity.rs, crates/rastro-storage-sqlite/src/automatic.rs, crates/rastro-storage-sqlite/migrations/202611071000_owner_intents.sql, crates/rastro-storage-sqlite/migrations/202611081000_intent_justification_end.sql, crates/rastro-storage-sqlite/migrations/202611091100_automatic_justification.sql, crates/rastro-storage-sqlite/tests/identity.rs, crates/rastro-connector-linear/, crates/rastro-extractor-linear/, crates/rastro-cli/tests/identity.rs

## Requirements

### Requirement: The owner's Person exists and resolves from me
`rastro identity me` SHALL create the owner's one `rastro:person/<id>` entity typed `rastro:Person`, `about me` SHALL resolve to it, and a second run SHALL return that same Person and record no new intent.

#### Scenario: me runs
- WHEN `rastro identity me` runs
- THEN a `rastro:person/<id>` entity typed `rastro:Person` exists and `about me` resolves to it

#### Scenario: me runs again
- WHEN `rastro identity me` runs again
- THEN it returns the same Person and records no new intent

### Requirement: A shared name is an ambiguous alias
Two Persons created with one name SHALL make that name an alias of both, so `about` on it SHALL fail as ambiguous, naming both.

#### Scenario: two Persons are named the same
- WHEN `rastro identity person create --name "Ana Silva"` runs twice
- THEN two distinct Persons exist and `about "rastro:name/Ana Silva"` returns the ambiguous-alias error listing both

### Requirement: A link names its Person and its intent
An owner link SHALL give its account an observed `rastro:held_by` claim naming the Person, justified by the owner intent alone, and `why` on that link SHALL give the intent's id, time and issuer.

#### Scenario: the owner links an account
- WHEN the owner links `github:login/<x>` to a Person
- THEN `about github:login/<x>` shows `rastro:held_by` that Person, and `why` on the link shows status `observed` with an `owner_intent` justification giving the intent's id, time and issuer

### Requirement: Undoing a link removes it and records a cannot-link
Undoing a link SHALL remove its `rastro:held_by` claim, record a cannot-link for the pair, and `rastro identity history <entity>` SHALL list the link, the undo and the cannot-link in acceptance order.

#### Scenario: the owner undoes a link
- WHEN the owner undoes that link
- THEN the `held_by` claim is gone, a cannot-link for the pair is recorded, and `rastro identity history <entity>` lists the link, undo and cannot-link in acceptance order

### Requirement: A merge groups both Persons' claims
Merging two Persons SHALL show the claims of both Persons' accounts on either one with equivalent claims grouped, and undoing the merge SHALL separate them and record a cannot-link.

#### Scenario: the owner merges two Persons
- WHEN the owner merges two Persons
- THEN `about` on either shows the claims of both Persons' accounts with equivalent claims grouped

#### Scenario: the owner undoes the merge
- WHEN the merge is undone
- THEN they separate and a cannot-link is recorded

### Requirement: An invalid intent is refused and stores nothing
An invalid intent SHALL be refused with an operation error naming the reason, and no intent SHALL be stored.

#### Scenario: the intent is invalid
- WHEN an intent is a `held_by` from a Person, a merge of two accounts or of an entity with itself, a target that doesn't exist, or an undo of a missing or already-undone intent
- THEN it's refused with an operation error naming the reason, and no intent is stored

### Requirement: An issuer cannot act on what it cannot see
An intent issued from a context whose scopes cannot see its target SHALL be refused at the storage seam, and nothing SHALL be stored.

#### Scenario: a scoped issuer names a hidden target
- WHEN an intent is issued from a context whose scopes can't see its target
- THEN it's refused (storage seam)

### Requirement: A write shows in the next scoped read
An intent written after a scoped read was cached SHALL be reflected by the next scoped read.

#### Scenario: a write follows a cached scoped read
- WHEN an intent is written after a scoped read was cached
- THEN the next scoped read reflects it

### Requirement: A sync records the accounts it observed
A GitHub sync SHALL record the token owner's account, its verified emails with unverified ones excluded, and each author account's profile name, and `about` on the token owner's account SHALL show its `github:email/…` aliases.

#### Scenario: the GitHub connector syncs
- WHEN the GitHub connector syncs
- THEN evidence records the token owner's account, its verified emails (unverified ones excluded) and each author account's profile name, and `about` on the token owner's account shows its `github:email/…` aliases

### Requirement: Unreadable emails are a coverage gap
A sync whose token cannot read `/user/emails` SHALL record a coverage gap saying the emails could not be read and SHALL still succeed.

#### Scenario: the token cannot read the owner's emails
- WHEN the GitHub token can't read `/user/emails`
- THEN coverage says the emails couldn't be read, and the sync still succeeds

### Requirement: The GitHub token owner holds me
When the owner's Person exists, a sync that records the GitHub token owner SHALL hold that account by the owner's Person with a token-owner justification, with no owner command.

#### Scenario: a sync records the GitHub token owner
- WHEN `me` exists and a sync records the GitHub token owner
- THEN that account is `held_by` `me` with a token-owner justification, with no owner command

### Requirement: An equal verified email links the account
An account whose verified email equals, under ASCII case folding, a verified email of an account already held by a Person over an overlapping interval SHALL be held by that Person, justified by both email observations, and SHALL make no link and create no Person when no held account has that email.

#### Scenario: a verified email matches a held account's
- WHEN account B has a verified email equal (ASCII case-folded) to a verified email of an account already linked to Person P, over an overlapping interval
- THEN B is `held_by` P, justified by both email observations

#### Scenario: the verified email matches nothing
- WHEN no linked account has that email
- THEN no link is made and no Person is created

### Requirement: Equal emails must overlap in time to link
Equal verified emails observed only over non-overlapping intervals SHALL make no link.

#### Scenario: the equal emails never overlap
- WHEN the equal emails were observed only over non-overlapping intervals
- THEN no link exists

### Requirement: A guest never links automatically
A matching email that belongs to a Linear guest SHALL make no automatic link.

#### Scenario: the matching email is a guest's
- WHEN a matching email belongs to a Linear guest
- THEN no automatic link is made

### Requirement: A cannot-link blocks the automatic link
A cannot-link between an account and a Person, directly or through a `same_as` chain, SHALL block the automatic link between them, and the owner SHALL still be able to link them explicitly.

#### Scenario: a cannot-link stands between the account and the Person
- WHEN a cannot-link exists between B and P (directly or through a `same_as` chain)
- THEN the automatic link isn't made

#### Scenario: the owner links them anyway
- WHEN the owner then links them explicitly
- THEN the link exists

### Requirement: An unobserved email closes its link
When the email behind an automatic link is no longer observed on the account, the link's interval SHALL close and `about` SHALL show it as historical.

#### Scenario: the email disappears from the account
- WHEN the email is no longer observed on B
- THEN the link's interval closes and `about` shows it as historical

### Requirement: The Linear key owner holds me
When the owner's Person exists, a Linear sync that records the API key's user SHALL hold that Linear account by the owner's Person with a token-owner justification.

#### Scenario: a Linear sync records the key's user
- WHEN `me` exists and a Linear sync records the API key's user
- THEN that Linear account is `held_by` `me` with a token-owner justification

### Requirement: A Linear sync records users and issues
A Linear sync SHALL record each user as a `linear:user/<uuid>` entity typed `tracker:Account` with `email` and `name` aliases and its active state, each of the viewer's involved issues as a `tracker:Issue` entity with creator and assignee claims, and `rastro search` SHALL find issue text with citations.

#### Scenario: a Linear source syncs against the scripted API
- WHEN a Linear source syncs against the scripted API
- THEN users become `linear:user/<uuid>` entities typed `tracker:Account` with `email` and `name` aliases and their active state, the viewer's involved issues become `tracker:Issue` entities with creator and assignee claims, and `rastro search` finds issue text with citations

### Requirement: A deactivated user keeps its links
A deactivated Linear user SHALL show as deactivated on its account, and its existing links SHALL remain, marked deactivated.

#### Scenario: a Linear user is deactivated
- WHEN a Linear user is deactivated
- THEN its account shows it, and its existing links remain, marked deactivated

### Requirement: A missing Linear key is an auth failure
A Linear connection with no API key SHALL fail with an auth failure shown in coverage, and the key SHALL never appear in the database or the config.

#### Scenario: no Linear API key is available
- WHEN no Linear API key is available
- THEN the pair fails with an auth failure shown in coverage, and the key never appears in the database or config

### Requirement: One folded name makes a candidate
A GitHub account and a Linear account whose full names are equal after case and accent folding, and that aren't linked to one Person, SHALL appear in `rastro identity candidates` as a pair with reason `same name`.

#### Scenario: two accounts carry one folded name
- WHEN a GitHub account and a Linear account have the same full name after case and accent folding and aren't linked to one Person
- THEN `rastro identity candidates` lists the pair with reason `same name`

### Requirement: An email matching a login makes a candidate
A pair whose Linear email's local part equals the GitHub login SHALL be listed as a candidate with reason `email matches login`.

#### Scenario: the email's local part is the login
- WHEN a Linear email's local part equals a GitHub login
- THEN the pair is listed with reason `email matches login`

### Requirement: A yes holds both accounts by one Person
The owner's yes to a candidate SHALL hold both accounts by one Person — the existing holder, or a new Person named from the candidate — and the pair SHALL no longer appear.

#### Scenario: the owner answers yes
- WHEN the owner answers yes to a candidate
- THEN both accounts are `held_by` one Person (the existing one, or a new one named from the candidate) and the pair no longer appears

### Requirement: A no records a cannot-link
The owner's no SHALL record a cannot-link and the pair SHALL never appear again, including after `claims rebuild`.

#### Scenario: the owner answers no
- WHEN the owner answers no
- THEN a cannot-link is recorded, the pair never appears again, including after `claims rebuild`

### Requirement: One namespace or a bot is never a candidate
A pair within one namespace, or that includes a `code:Bot`, SHALL never be a candidate.

#### Scenario: the pair shares a namespace or has a bot
- WHEN a pair is within one namespace or includes a `code:Bot`
- THEN it's never a candidate

### Requirement: About me lists accounts and their claims
`about me` SHALL list the Person, each linked account with its reason, and the accounts' claims, each citing its own account, with equivalent claims grouped and counted once toward `limit`.

#### Scenario: me has a GitHub and a Linear account
- WHEN `about me` runs with linked GitHub and Linear accounts
- THEN the result lists the Person, each linked account with its reason, and the accounts' claims, each citing its own account, with equivalent claims grouped and counted once toward `limit`

### Requirement: An account's about shows its Person
`about` on an account that a Person holds SHALL include `rastro:held_by` that Person.

#### Scenario: about names a linked account
- WHEN `about` names an account linked to a Person
- THEN the result includes `rastro:held_by` that Person

### Requirement: A Person is visible only with a visible account
A scoped read that can see none of a Person's linked accounts SHALL answer for that Person exactly as for an unknown entity, and `about me` SHALL show only the accounts and claims the scope sees.

#### Scenario: the scope hides every linked account
- WHEN a scoped read can't see any of a Person's linked accounts
- THEN `about` on that Person equals the unknown-entity answer, and `about me` in any scope shows only the visible accounts and claims

### Requirement: A scoped read equals a per-scope rebuild
Every scoped answer SHALL equal a rebuild from that scope's visible inputs, including a link visible only where both of its ends are.

#### Scenario: the random scope-set oracle runs
- WHEN the random scope-set oracle runs with intents and automatic links
- THEN each scoped answer, including links visible only where both ends are, equals a per-scope rebuild

### Requirement: About me fails rather than answer partially
An `about me` that would exceed the work budget SHALL fail with the budget error instead of answering partially.

#### Scenario: about me would exceed the work budget
- WHEN `about me` would exceed the work budget
- THEN it fails with the budget error instead of a partial answer

### Requirement: The owner's database answers about me
On a copy of the owner's database, after the owner synced `svc-platform` and Linear and ran `rastro identity me`, `about me` SHALL list the owner's GitHub and Linear accounts and `rastro identity candidates` SHALL return without error.

#### Scenario: the ignored live test runs on the owner's copy
- WHEN the ignored live test runs on a copy of the owner's database after the owner synced `svc-platform` and Linear and ran `rastro identity me`
- THEN `about me` lists the owner's GitHub and Linear accounts, and `rastro identity candidates` returns without error

### Requirement: Only account types may be linked
A link, owner or automatic, whose account side names a type no vocabulary declares an account SHALL be refused — an owner link with an operation error naming the reason, an automatic link never made — while `code:Account` and `tracker:Account` SHALL link as before.

#### Scenario: a link names something that is not an account
- WHEN a link, owner or automatic, names an account side whose type no vocabulary declares an account (a `code:PullRequest`, a `code:Repository`)
- THEN it's refused (an owner link with an operation error naming the reason; an automatic link is never made), while `code:Account` and `tracker:Account` link as before

### Requirement: An undone claim stays superseded
Undoing a link or a merge, or naming a Person again, SHALL keep the undone claim or the older name as `superseded` with its interval closed at that time, `about` SHALL show it as historical, and the older name SHALL still resolve to the Person.

#### Scenario: an undo leaves the claim behind as superseded
- WHEN the owner undoes a link or a merge, or names a Person again
- THEN the undone claim (or the older name) stays as `superseded` with its interval closed at that time, `about` shows it as historical, and the older name still resolves to the Person

### Requirement: Undoing a newer name leaves a gap
Undoing a Person's newer name SHALL leave the older name's interval closed when the newer name was given and open again from the undo.

#### Scenario: the newer name is undone
- WHEN the owner undoes a Person's newer name
- THEN the older name shows a gap: closed when the newer name was given and open again from the undo

### Requirement: An intent records its issuer's scopes
An intent SHALL record its `issuer_context` as the issuing surface's configured scopes, which `rastro identity history` SHALL show.

#### Scenario: the owner issues an intent
- WHEN an intent is issued
- THEN its `issuer_context` lists the issuing surface's configured scopes, and `rastro identity history` shows them

### Requirement: An app user is a bot and never links
A Linear sync that reads an app (integration) user SHALL type it `tracker:Bot`, never `tracker:Account`, and a link to it SHALL be refused.

#### Scenario: a sync reads an app user
- WHEN a Linear sync reads an app (integration) user
- THEN it's typed `tracker:Bot`, never `tracker:Account`, and a link to it is refused

### Requirement: Cannot-linked accounts hold only their own intervals
Two accounts refused to each other that each qualify for an automatic link to one Person over non-overlapping intervals SHALL each hold that Person over its own interval, and neither SHALL over any overlap.

#### Scenario: two cannot-linked accounts qualify over separate intervals
- WHEN two cannot-linked accounts each qualify for an automatic link to one Person over non-overlapping intervals
- THEN each holds that Person over its own interval, and over any overlap neither does
