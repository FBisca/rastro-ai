# Plugin model

Status: agreed 2026-11-04. North star: `../concept.md` §3 and §4.

## Decisions
1. **Plugins request, the core decides.**
   - No plugin writes knowledge or runs an Act directly.
   - Plugins submit evidence, claims, proposals, questions and action requests.
   - The core checks each one against vocabularies, scopes, levels and rules before recording or running it.
2. **Kinds of plugin**

| Kind | Does | Submits / receives |
|---|---|---|
| Connector | Reads a source | Evidence and coverage |
| Extractor | Turns evidence into claims, deterministically or through an agent tool | Claims with justification (hypothesis unless observed) |
| Vocabulary | Defines types, predicates and their language | Definitions |
| Workflow | Runs a job over the knowledge | Proposals, questions, action requests |
| Action provider | Exposes actions, each with its reach and reversibility | Executes Action runs when the core allows it |
| Renderer | Shows views, takes the owner's input | Receives events; sends intents |
| View | A query plus a layout for a renderer | Shareable |
| Policy pack | Rules and level defaults | Can only lower levels or restrict, never grant Act |
| Agent tool adapter | Lets any agent runtime do judgment | Structured output, validated against a schema |

3. **Connector and Extractor are separate kinds.** Reading a source and interpreting it have different trust levels and evolve separately (swap the extractor, replay the evidence).
4. **Every plugin declares a manifest:**
   - identity: name, version, author;
   - which kinds it provides;
   - permissions: the scopes it reads, what it may submit, the actions it exposes;
   - triggers;
   - dependencies: the vocabularies it uses, and the *capabilities* it needs from an agent tool (never a named tool).
5. **Permissions work like a phone app's.** The owner is asked at install. An update shows what changed in its permissions. The owner can revoke any permission at any time.
6. **Quarantine.** A plugin that reads untrusted content (mail, chat, the web) can never hold action permissions.
7. **Plugins talk only through the graph.**
   - The core emits events: new evidence, a claim changed, a retraction, a coverage gap, a proposal decided.
   - Workflows start from **triggers**: a schedule, on demand, or a graph event.
   - No plugin or workflow calls another directly. Composition happens through events and claims.
8. **Two graphs kept apart.** The execution graph (what runs, and when) is separate from the knowledge graph (what's known).
9. **Measured by default.** Every plugin run is logged: cost, output, and the share of its output accepted or ignored. Usefulness is visible per plugin.
10. **Agent tools are plugins.** Claude Code, Codex, a local model or anything else; none is special to the core.
11. **Distribution.**
    - There's a registry, and plugins install in one step.
    - A starter pack bundles the standard vocabulary, common connectors and default workflows.
    - How the registry is governed is open. See `../open-questions.md`.

## Added 2026-11-04 (roadmap slice 1)
12. **The manifest declares; grants narrow.** Registering a source grants a subset of the manifest and binds its scope. The core rejects anything outside the grant. Format: `../adr/0004-plugin-manifest-and-grants.md`.
13. **A connector that reads untrusted content uses only fixed read operations.** Nothing it reads can choose what it calls. Running it as a child process is not a sandbox, and Rastro doesn't claim OS isolation.

## Added 2026-11-04 (roadmap slice 3)
Architecture: `../adr/0008-extractor-role-and-protocol.md`.
14. **The Extractor role is live,** deterministic and out of process, on the connector's supervision. The core validates every claim: delivered evidence, trust class, span, an extraction rule declared deterministic, and the vocabulary (§1). Hypothesis claims wait for slice 5.
15. **An extractor is bound to sources by config,** with a default per connector (GitHub: `rastro-extractor-github`). It reads only the evidence of its bound sources; an installed but unbound one never runs; a bound but missing one reads `not_installed`. The connector's manifest names its default extractor; the source's config binds it.
    If extraction fails after ingestion, `sync` exits non-zero, the source's coverage stays as read, its extraction state reads `failed`, and reads serve the last good projection.
16. **One package may provide several kinds** (§4). The GitHub extractor provides `extractor` and `vocabulary`; that is no exception to `../adr/0001-rust-core-out-of-process-plugins.md`.
17. **Extractor runs are logged** as runs (§9) with plugin id, version and counts, apart from connector coverage.

## Added 2026-11-05 (knowledge graph deep dive)
Architecture: amendments of 2026-11-05 to `../adr/0008-extractor-role-and-protocol.md`.
18. **Direction for slice 5: derivation is a replaceable plugin.** A plugin submits claims with declared premises (for example "lift a promise to the Person"); the core does generic validation and maintenance only. No fixed core rule registry, since domain rules in the core contradict §3 and `vocabulary.md` §1. Derivation is an extractor role, not a new kind of plugin, so the glossary gets no new term. It needs a protocol message first; see the slice 5 plan.
19. **Retirement and several extractors (amends §15).** One binding per source stays. When an extractor's version changes, its old justifications are replaced atomically once the replay completes (`knowledge.md` §40). Several extractors on one field give one claim with several labelled justifications (`knowledge.md` §39).
20. **Location context (clarifies §14).** An extractor may receive companion fields of a changed record, within a bound: same source, scope and location, evidence no newer than the batch's high-water mark, and recomputable on replay. The ids actually delivered are kept for replay. This is recorded in the amendment to ADR 0008.

## Added 2026-11-05 (roadmap slice 4)
Plan: `../plans/2026-11-05-second-source-one-identity.plan.md`.
21. **A connector manifest declares its source options.** Each option carries a name, whether it is required, a pattern and a help text. `rastro source add --connector <id> --option key=value` validates against the declared set: an option the manifest doesn't declare, or a missing required one, is refused with an error naming the declared options, and nothing is saved. GitHub's `--repo` stays as a shorthand for its declared option, and the check moves into the GitHub manifest.
22. **The Linear API key is read at sync.** From `LINEAR_API_KEY` or the keychain item `rastro-linear`, like the GitHub token. It is never stored in the database or the config, and a sync without a key fails with an auth failure in coverage (`knowledge.md` §18).
