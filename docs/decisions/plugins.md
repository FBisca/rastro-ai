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
    - *Amended 2026-11-05 (strict checking):* a manifest that declares no options refuses any option. A connector that isn't installed yet may be added, but the source's options are checked at sync before binding and fail as `protocol`. GitHub falls back to the bundled declaration when the installed manifest declares none. A stored source is checked the same way at sync, so a hand-edited option never reaches the connector. A pattern must match the whole value and must parse on its own; an invalid pattern fails the manifest. Build-level; confirmed by the owner (2026-11-05).
22. **The Linear API key is read at sync.** From `LINEAR_API_KEY` or the keychain item `rastro-linear`, like the GitHub token. It is never stored in the database or the config, and a sync without a key fails with an auth failure in coverage (`knowledge.md` §18).
23. **The Linear connector reads what involves the API-key user.** Workspace users (id, name, display name, email, active) and the issues the API-key user created, is assigned or subscribes to (identifier, title, description, state, creator, assignee, team, timestamps). Comments and issue-to-PR links wait for slice 5.
24. **One Linear workspace for now (2026-11-05, extends §21 and §22).** A second source for the `linear` connector is refused with an error naming the existing one, because one key (`LINEAR_API_KEY`, keychain `rastro-linear`) would read the same workspace into another scope. Per-source keys wait until needed. Plan D33 (owner Q12); the plan is `../plans/2026-11-05-second-source-one-identity.plan.md`.
    - *Amended 2026-11-05 (sync enforcement):* `single_source` is also enforced at sync. A second source for a single-source connector fails as `protocol`, naming the other source, before the connector runs; the first source still syncs. This covers hand-edited and pre-upgrade configs, as §21 does for stored options. Plan D42 (owner Q21): `../plans/2026-11-05-second-source-one-identity.plan.md`.
25. **A token's owner is recorded as kind `viewer` (2026-11-05, extends §22 and §23).** A connector records the account its token belongs to as an item of kind `viewer` at the per-source location `<namespace>:viewer`, field `/id`. It is one slot; a different owner replaces it. Both connectors do this now: GitHub records `/id` and `/type`, keeping an original trimmed to `{id, type}` when originals are retained; Linear records `/id` and `/app`. Extractors emit `rastro:token_owner` from this evidence; the core derives the link from that claim (`../adr/0010-identity-and-owner-intents.md`, plan D44). Plan D34 (owner Q13); the plan is `../plans/2026-11-05-second-source-one-identity.plan.md`.
26. **Coverage reports may include gaps (2026-11-05).** An optional list names selections the connector couldn't read and their typed reasons. A gap doesn't fail an otherwise successful run. Gaps are stored additively and shown by `coverage` in CLI and MCP. GitHub reports unreadable or rate-limited verified-email and author-profile reads as gaps. A failed email read writes no snapshot and preserves the previous one. Plan D45: `../plans/2026-11-05-second-source-one-identity.plan.md`.

27. **GitHub records verified-email snapshots and author names (2026-11-06).** Alongside the viewer (§25), the connector records the token owner's verified addresses at `/emails/verified`, including an empty snapshot, and author profile names, with retained originals trimmed to `{id, name}`. Snapshot evidence and exact address spans: `../adr/0010-identity-and-owner-intents.md`. Plan D46 and D49.
28. **Connectors don't query stored data (2026-11-11).** A connector gets only the structured known state the core puts in its run binding; it never reads Rastro's store. A workflow asking the core for a targeted read ("directed reads") is parked until a slice needs it.

## Added 2026-11-13 (agent tool adapter)
Architecture: `../adr/0012-agent-tool-adapter.md`.
29. **The agent tool adapter is live (clarifies §2, §4, §10).** A plugin of kind `agent-tool-adapter` lets an agent runtime do judgment over text the core sends it. Its manifest declares the kind and `agent_tool = { capabilities = [...] }`, the capabilities it needs from an agent tool; the one today is `judge_text`. It never names a tool: the owner picks the runtime by installing its adapter.
30. **A manifest that names a tool, or doesn't declare the kind, is refused before anything runs.** So is one with no capability, a capability the core doesn't know, or an action permission (§6: it reads untrusted text). The run fails with the reason; the adapter is never started.
31. **The core asks, the adapter proposes, the core checks.** The core sends its own instructions, the content and the fields it expects, and keeps only the candidates that pass its own checks (`meetings.md` §4). A response that breaks the expected shape fails the run and leaves the last good results as they were, stale.
32. **The owner binds one by plugin id** in `config.toml` (`[commitment_adapter]`). Binding it is the authorization to send transcripts to it, and nothing else is sent to it. It starts with a cleared environment plus the basics, what its manifest lists, and the location of Rastro's home.
33. **The reference adapter drives Claude Code,** with every tool, setting, MCP server and slash command off. It is installed by hand until slice 15; another runtime is another adapter the owner writes or asks for.
