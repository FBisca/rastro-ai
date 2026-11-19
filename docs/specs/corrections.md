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
