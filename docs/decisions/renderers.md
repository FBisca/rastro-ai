# Renderers and views

Status: agreed 2026-11-04. North star: `../concept.md` §6. Related: `interruptions.md`.

## Decisions
1. **Renderers hold no state.** They show **views** (a query plus a layout plus a renderer, defined by the owner). The owner's input goes back as typed **intents**:
   - `confirm`, `accept`, `edit`, `reject`
   - `answer`, `undo`, `stop`
   - `promote`, `missed`
2. **The owner answers once, anywhere.** The same proposal or question can appear on several surfaces. Answered on one, it disappears from all of them.
3. **Scopes apply to renderers.** For example, a lock-screen notification can be denied the `health` scope.
4. **Views are shareable plugins,** e.g. a "1:1 prep" view or a "weekly review" view.
5. **Intents target a claim ID plus the revision the owner saw.** If the claim changed since, the owner sees a conflict instead of a silent overwrite.
6. **Renderers report what they showed**, so "ignored" can be measured (see `learning.md` §3).

## Notes vault (e.g. Obsidian)
The vault is the owner's writing space plus a disposable view of the graph. It is never an authority.
1. **One writer per path.** The owner's notes are theirs; the renderer never rewrites them. Generated views live in their own clearly named folder, stamped with entity IDs, the time they were generated, and links to their sources.
2. **Link, don't copy.** Owner notes embed or link generated views instead of retyping a status. This avoids duplication and drift.
3. **Only ticks and structured fields are intents or confirmations.** Ordinary metadata isn't automatically an intent.
4. **The owner's prose enters only through an opt-in connector, "Owner notes":**
   - Each note revision becomes evidence, and extractors propose hypotheses from it.
   - It never auto-confirms anything.
   - Contradictions with the graph show up for review.
   - Notes are enrolled one at a time.
5. **That connector never reads generated files or embedded views.** Otherwise Rastro would ingest its own claims as fresh evidence.
6. **Rastro never "fixes" the owner's prose automatically.**
7. **The vault isn't the backup.** The export is (see `sharing.md`).
8. **Main risk:** prose becomes a second authority that people believe while it goes stale. Mitigations: links instead of copied status, a contradictions view, opting in note by note.

## Herdr
Rastro can be a Herdr mod or layout (https://herdr.dev/gallery/): a pane showing the queue of proposals and questions, and a "needs you" status reported through herdr's Program Status Protocol (OSC 7501).

## Added 2026-11-05 (knowledge graph deep dive)
7. **Amends §5: intents can target a set of claims.** Once a merge exists (roadmap slice 4), a group of equivalent claims is targeted as its member claim ids, each with the revision seen (`knowledge.md` §35). A change to any member shows as a conflict. Group revision is a hash of the sorted (member id, revision) pairs.
8. **Revision is computed from visible state only** (`knowledge.md` §45). A renderer's stale check still uses it; the durable binding of a confirmation or retraction uses the lifecycle state instead (`knowledge.md` §42).
