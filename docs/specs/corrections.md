# Corrections

Paths: crates/rastro-core/src/identity.rs, crates/rastro-core/src/claims.rs, crates/rastro-core/src/knowledge.rs, crates/rastro-core/src/queue/, crates/rastro-storage-sqlite/src/, crates/rastro-operations/, crates/rastro-cli/src/, schema/

The owner confirms, retracts or fixes any claim in the projection from the CLI (`rastro claim confirm|reject|edit <claim-id> --seen <revision>`, and `rastro claim undo <intent-id>`). Each correction is an owner intent (`../adr/0010-identity-and-owner-intents.md`): insert-only, outside the rebuildable projection, and replayed by `claims rebuild`. A correction names the revision the owner read, and a claim that changed since is refused with nothing recorded (`../decisions/renderers.md` §5). Writes are CLI-only and the MCP server stays read-only (`../decisions/surfaces.md` §37).

A correction is bound to the claim's lifecycle fingerprint (`../adr/0014-lifecycle-fingerprint.md`): a retraction holds while the occurrence it saw holds, survives corroboration, and lapses when that occurrence closes, returning the claim marked "earlier retracted" with a queue question; a confirmation goes stale when its evidence ends (`../decisions/knowledge.md` §42). A retracted premise ends the derivations resting only on it: automatic identity links, and an accepted commitment's slots through its anchor. The queue stops counting a retracted statement as known. `about` and `why` answer as of a past time, reconstructed from the evidence captured and the intents issued by then, under today's rules (`../decisions/knowledge.md` §47).

## Switch day

### The owner's own `about me` — 2026-11-18

The owner installed the build from `main` on the machine that holds the real sources and backed up
the store. The first write applied the `claim_statuses` migration, and `rastro claims rebuild`
replayed every source and intent: 9,170 claims. The last sync had run on 2026-11-13, so `about me`
still showed six of the owner's pull requests as `open` that GitHub had merged that day.

The owner fixed one and retracted another:

- `rastro claim edit` on `code:state "open"` of rastro-ai#122 ("docs(roadmap): land the retro
  roadmap…"), object `closed`. One intent retracted the stale claim and asserted `code:state
  "closed"`, justified by the edit alone; `about` shows the pull request closed.
- `rastro claim reject` on `code:state "open"` of agent-runtime#303 ("docs(plan-amend): archive the
  plan"). The claim left `about`; `why` lists it `retracted`, with the reject intent and the GitHub
  evidence it overrides, both current.

The owner confirmed one relied on: `rastro claim confirm` on `code:state "open"` of rastro-ai#102
("sync is slow and silent…"), checked open on GitHub that day. `about` lists it `confirmed`, with
two supports: the evidence and the confirmation.

`rastro about me --as-of 2026-11-18T12:00:00Z` answered from the evidence and intents by then and
said so, naming the extractor versions it used. None of the three corrections had been issued yet.

The next sync should close both stale occurrences; the retraction on agent-runtime#303 then lapses,
and the queue asks whether it is still wrong.

## Requirements

### Requirement: The owner confirms a claim
`claim confirm` SHALL record one `confirm` intent on a claim the owner can see, bound to the lifecycle state of the revision the owner saw, and the claim SHALL then read `confirmed`.

#### Scenario: a confirmed claim reads confirmed
- WHEN the owner confirms an observed claim naming its current revision
- THEN `about` shows the claim with status `confirmed`, and `why` lists a confirmation justification citing the intent's id and time

#### Scenario: a stale revision is a conflict
- WHEN the owner confirms a claim naming a revision that is no longer the claim's current one
- THEN the command fails naming the claim and its current revision, and no intent is recorded

#### Scenario: a confirmed member leads its group
- WHEN a merged Person's group holds a confirmed member and an observed member for one predicate
- THEN the group lists the confirmed member before the observed one

### Requirement: The owner retracts a claim
`claim reject` SHALL record one `reject` intent on a claim the owner can see, and the claim SHALL then be left out of every current read in every scope that can see a justification of it.

#### Scenario: a retracted claim leaves about
- WHEN the owner rejects an observed claim naming its current revision
- THEN `about` no longer lists the claim, and `why` on its id shows status `retracted` with the reject intent

#### Scenario: a scoped read hides it too
- WHEN a claim is rejected and an MCP server launched with a scope that sees it reads `about`
- THEN the claim is absent from that scoped result

#### Scenario: an owner link goes through undo
- WHEN the owner rejects a held_by or same_as claim that an owner `link` intent made
- THEN the command fails naming `identity undo` and the link intent, and no intent is recorded

### Requirement: The owner fixes a claim's object
`claim edit` SHALL record one `edit` intent that retracts the target claim and asserts a claim with the same subject and predicate and the new object, justified by that intent alone.

#### Scenario: the fixed claim replaces the old one
- WHEN the owner edits a claim's object
- THEN `about` lists a claim with the new object and not the old one, and `why` on the new claim cites the edit intent as its only justification

#### Scenario: an object outside the range is refused
- WHEN the new object breaks the predicate's range
- THEN the command fails naming the predicate and its range, and no intent is recorded

### Requirement: A retraction ends only what rests on it
A retraction SHALL end every derivation whose premise it retracts, and SHALL leave a claim standing while another justification of it is live.

#### Scenario: an automatic link loses its premise
- WHEN the owner rejects the verified-email claim an automatic held_by link rests on
- THEN `about` on the Person no longer lists that account as held

#### Scenario: an independent justification keeps the claim
- WHEN the rejected premise's automatic link is also justified by an owner `link` intent
- THEN `about` still lists the account as held, justified by the owner link

#### Scenario: a commitment's anchor takes its slots
- WHEN the owner rejects the anchor claim of an accepted commitment
- THEN every slot claim that only that anchor justified is absent from `about`, and a slot with its own observed justification is still listed

### Requirement: A retracted commitment is no longer known
The queue's known-first check SHALL NOT count a retracted claim's statement as known, and SHALL mark a proposal that expresses it as earlier retracted.

#### Scenario: new evidence proposes it again
- WHEN an accepted commitment's claims are retracted and a pending proposal expresses the same statement
- THEN the queue shows that proposal, marked earlier retracted, instead of counting it as known

### Requirement: A correction follows its claim's lifecycle state
A retraction or confirmation SHALL hold while the claim's lifecycle fingerprint is the one it was bound to; a changed fingerprint SHALL lapse a retraction and leave a confirmation standing but stale.

#### Scenario: a lapsed retraction returns marked
- WHEN a retracted claim's bound occurrence closes
- THEN `about` lists the claim again with its evidence-derived status, marked earlier retracted

#### Scenario: new corroboration keeps it retracted
- WHEN a second occurrence from another source corroborates a retracted claim
- THEN `about` still leaves the claim out

#### Scenario: a confirmation goes stale
- WHEN the only evidence justification of a confirmed claim ends
- THEN `about` lists the claim as `confirmed`, marked stale, and `why` shows the ended justification in its history

### Requirement: A lapsed or stale correction asks the owner
The queue SHALL hold one question for each lapsed retraction and each stale confirmation the caller can see, and answering it SHALL record one intent.

#### Scenario: the queue asks
- WHEN a retraction lapses
- THEN the queue holds one question naming the claim, its current status and the earlier reject intent

#### Scenario: yes retracts it again
- WHEN the owner answers that question yes
- THEN `about` leaves the claim out again, and the question is gone from the queue on every surface

### Requirement: The latest intent wins, and undo removes only its own
A claim's correction status SHALL come from its latest valid intent, and `claim undo` SHALL remove only the named intent's contribution.

#### Scenario: a confirm reverses a retraction
- WHEN the owner confirms a claim they had rejected
- THEN `about` lists the claim as `confirmed`

#### Scenario: undo restores the earlier state
- WHEN the owner undoes a reject issued after a confirm on the same claim
- THEN `about` lists the claim as `confirmed`, and `why` shows the undone reject in its history

### Requirement: Corrections replay at rebuild
`claims rebuild` SHALL replay every correction intent so that the rebuilt projection equals the incremental one.

#### Scenario: rebuild keeps corrections
- WHEN the projection holds confirmed, retracted and fixed claims and the owner runs `claims rebuild`
- THEN `about` and `why` return the same claim ids and statuses as before the rebuild

### Requirement: Reads as of a past time
`about` and `why` with `as_of` SHALL reconstruct the answer from the evidence captured by that time and the intents issued by it, under today's rules, and SHALL say so.

#### Scenario: the answer before a correction
- WHEN the owner reads `about` as of a time before they rejected a claim
- THEN the claim is listed with its status before the rejection, and the result states it is a reconstruction naming the extractor and vocabulary versions it used

#### Scenario: a source not read yet is named
- WHEN `as_of` is earlier than any evidence captured from a source the entity has claims from
- THEN the result names that source as not yet read at that time, instead of answering with no claims

#### Scenario: MCP answers the same
- WHEN an agent calls the MCP `about` tool with `as_of`
- THEN it returns the same claims and statuses as the CLI with the same `as_of`

### Requirement: Corrections are CLI writes in the catalog
The operation catalog SHALL define `claim.confirm`, `claim.reject`, `claim.edit` and `claim.undo` as CLI-only writes, and the MCP server SHALL list none of them.

#### Scenario: MCP stays read-only
- WHEN an agent lists the MCP server's tools
- THEN no correction write is listed, and `about` and `why` accept `as_of` in their input schemas

### Requirement: Corrections serve a real morning
The owner SHALL correct their real `about me` from the CLI on the switch day, using the corrections instead of remembering what is wrong.

#### Scenario: switch day
- WHEN the morning after the last code ticket merges, the owner reads `about me` on their own store
- THEN they reject or edit at least one wrong claim and confirm one, and the next `about me` shows each change, recorded in `docs/specs/corrections.md` under Switch day
