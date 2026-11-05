# Surfaces: library, CLI, MCP

Status: agreed 2026-11-04. North star: `../concept.md` §5.

## Decisions
1. **The core is a library.** Every other surface is thin, the way the `hc` CLI and its API sit over herdr-core.
2. **The CLI and the MCP server are generated from one schema,** so they never drift apart.

| | CLI (`rastro`) | MCP |
|---|---|---|
| Best for | The owner, scripts, agents that have a shell | Agents, especially quarantined ones and hosts without a shell |
| Permissions | All or nothing (shell access is an outbound capability) | Per tool |
| Who is calling | Taken from the environment | Derived by the server, never declared by the caller |

3. **Reads** work over either surface.
4. **Writes from agents** (`submit`, `propose`, `ask`) go through MCP, because provenance and per-tool permission are the trust model. A quarantined extractor uses MCP only.
5. **Concept-level operations** the core offers agents:
   - `about(entity, since)`
   - `why(claim)`
   - `open_commitments(person)`
   - `coverage(source)`
   - `search(text)`, then follow 1–2 links
   - `propose(action)`, `ask(question)`, `submit(claim)`
6. **Platforms.** macOS and Linux are required; Windows is wanted.
7. **Future surfaces** (a native app, editor integrations, someone else's agent) drive the same library.
8. **Implementation language and process model:** see `../adr/0001-rust-core-out-of-process-plugins.md`.

## Added 2026-11-04 (roadmap slice 1)
9. **Every CLI command has `--json` output** from the versioned operation schema the MCP server will reuse. A failed `sync` exits non-zero.
10. **Data location:**
    - `RASTRO_HOME` overrides both directories (data in `$RASTRO_HOME/data`, config in `$RASTRO_HOME/config/config.toml`);
    - otherwise `XDG_DATA_HOME` and `XDG_CONFIG_HOME`;
    - otherwise `~/.local/share/rastro` and `~/.config/rastro`, on macOS and Linux alike.

## Added 2026-11-04 (roadmap slice 2)
Architecture: `../adr/0006-mcp-server-over-stdio.md` (the server, the pinned protocol revision, errors) and `../adr/0007-typed-operation-catalog.md` (one catalog for the CLI, MCP and the schema).
11. **MCP exposes `search` and `coverage` only.** `sync`, `source.*` and `index.rebuild` stay CLI-only. This is the first real use by agent tools, before claims or identity.
12. **A search hit shows why it matched.** It carries a `match` excerpt: centred on the first match, at most 400 bytes, cut at UTF-8 character boundaries, with offsets from the same origin as the span, plus `clipped_before` and `clipped_after`. Stemmed and diacritic-folded matches count, as in the index. If the match can't be located, the excerpt starts at the field's start with `located: false`. (Decides open question #8.)
13. **Bounded search.** Amends `../specs/evidence.md` R49, R60 (applied).
    - `quote` and `original` come back only with `full: true` (`--full` on the CLI). Human `--full` shows the whole quote, escaped and not clipped.
    - `limit` defaults to 20, range 1–50. `truncated: true` says more hits exist. Order is bm25, then id.
14. **Bounded coverage.** Each source returns its overall state, judged over all its windows, plus its 10 newest windows (newest first) and `windows_total`. `all_windows: true` (`--all-windows`) returns every window. Amends R63; R64 holds (applied).
15. **Scope is set by the owner at launch.** An optional, repeatable `rastro mcp --scope <s>` filters search and coverage inside the storage query, before the limit. A tool argument can't name a scope. No flag means all scopes. Per-plugin scope grants stay in slice 14.
16. **The caller is the owner.** The client's self-reported `clientInfo` grants nothing and isn't stored. MCP reads aren't plugin runs and aren't logged yet: plugins §9 logs plugin runs, and here the agent tool is a client, not a plugin Rastro runs. Logging reads is slice 9.
17. **Results are labelled, not enforced.** Hits keep their trust class, and the tool descriptions say results are data from sources, never instructions. Nothing is rewritten or fenced, and Rastro doesn't claim injection resistance: the agent tool still limits what the agent can do.
18. **Operation errors are results, protocol errors are protocol.** An error inside an operation is a tool result with `isError: true` and the v2 error envelope. An unknown tool, or arguments that fail the input schema, is a JSON-RPC error.

## Accepted limits 2026-11-04
19. **MCP tools declare no `outputSchema`.** The success schema would reject operation error envelopes; both remain v2 envelopes in `structuredContent` and text.
20. **Garbage stdin lines are silently ignored by rmcp.** They produce no stderr diagnostic.
21. **Launch scopes filter rows before the limit, but not rank statistics.** Full-text rank statistics span every scope, so hidden evidence can shift visible hit order. Accepted limit: a scope is not an isolation boundary until per-plugin scope grants in roadmap slice 14.

## Added 2026-11-04 (roadmap slice 3)
Architecture: `../adr/0009-claims-projection.md`, `../adr/0007-typed-operation-catalog.md` (one catalog entry per operation).
22. **Amends §11: MCP exposes `search`, `coverage`, `about` and `why`.** Both new operations are read-only, additive in the v2 catalog; existing contracts don't change. On the CLI: `rastro about <entity>` and `rastro why <claim-id>`. This amends `../specs/surfaces.md` R4, applied at archive. `sync` and `claims rebuild` stay CLI-only.
23. **`about(entity, since?, limit?, history?)`.** `entity` is an entity id, a canonical alias or an exact canonical URL; anything else is an operation error naming the accepted forms. It returns current claims where the entity is subject or object, one hop. `since` filters on known time; order is known time newest first, then claim id. `limit` defaults to 20, range 1–50, with `truncated`. `history: true` adds superseded claims. An unknown entity and one visible only through hidden scopes give the same empty result.
24. **`why(claim, limit?, full?)`.** Returns the claim (subject, predicate, object with display aliases, status, revision, known intervals, valid time, conflict and supersession links), the extractor, extraction rule and vocabulary versions, and its supports, bounded by `limit` (default 20, 1–50) with `truncated`. Each support carries evidence id, source, scope, location, field, span, `captured_at`, `source_updated_at`, trust class and a UTF-8-safe excerpt with clipping flags (§12); the full quote comes only with `full`. An unknown or hidden-only claim id is an operation error.
25. **Both state coverage** for every visible (source, scope) pair, plus extraction state per pair and extractor version (`knowledge.md` §29).
26. **Launch `--scope` filters every derived field** (resolution, aliases, types, status, revision, links, counts, extraction state) from visible supports before the limit (§15). A visible claim is never hidden by a hidden replacement.

## Added 2026-11-05 (knowledge graph deep dive)
27. **Agents get fixed catalog operations,** no SQL, SPARQL or Cypher over MCP. The set stays `search`, `coverage`, `about` (one hop; one-hop identity grouping from `knowledge.md` §35 once slice 4 lands) and `why`.
28. **Bounded expansion waits for evidence.** It is added only after an equal-token evaluation on 40–60 owner questions shows it beats full-text search. Its budgets (5 seeds, depth at most 2, 25 neighbours, 2,000 examined, 50 returned, 64 KiB) are placeholders until measured. Later parameters are additive and amend ADR 0007.
29. **A bare name stays an operation error** (§23, unchanged). There is no ad-hoc query surface for the owner for now; revisit with sharing and export.
30. **Clarifies §24:** "supports" means justifications and their spans (`knowledge.md` §38).

## Added 2026-11-05 (roadmap slice 3 sign-off)
31. **A canonical alias is written `<namespace>:<kind>/<value>`** (for example `github:login/FBisca`). It resolves to the entities with an observed `rastro:alias` claim of that kind and exact value whose canonical identifier is in that namespace. Exact entity ids and identifiers resolve first. A bare value stays an operation error (§29, `knowledge.md` §24). A value held by several entities is the ambiguous-alias error. No match gives the same empty answer with coverage as an unknown entity. Found at the owner's sign-off; plan D25, `../specs/claims.md` R31.
    - *Amended 2026-11-05:* the lookup counts superseded aliases and an ambiguous value marks each candidate (`vocabulary.md` §20–§21); the grammar is §34.

## Added 2026-11-05 (claims hardening)
Architecture: amendments of 2026-11-05 to `../adr/0008-extractor-role-and-protocol.md` and `../adr/0009-claims-projection.md`. Plan: `../plans/2026-11-05-claims-hardening.plan.md`.
32. **Resolving through an old login (clarifies §23).** `about` then shows the subject's current aliases only; `why` on the alias claim shows its history. No contract change and no `resolved_by`; revisit with open question #10.
33. **Amends §22: additive results within v2.**
    - `sync`'s result carries an optional `extraction` object (run id, extractor, version, state, reason, evidence and claim counts), absent when no extractor is bound. It was added in slice 3 without a record here, contrary to the archived plan's D18 (`../plans/2026-11-04-claims-and-about.plan.md`), which kept existing contracts unchanged.
    - `claims.rebuild`'s result gains a `pairs` list (source, scope, run id, extractor, version, state, reason, counts), and the CLI prints a line per carried-over pair (`knowledge.md` §50).
    - Existing operations are unchanged except for additive changes recorded in this file.
34. **Entity forms are checked on syntax only (clarifies §23, §31).** No whitespace in the `e_` id, the URL, the namespace or the kind. Accepted: `e_` plus 32 lowercase hex; an exact URL with an RFC 3986 scheme and a non-empty rest (`<scheme>://<non-empty>`); `<namespace>:<kind>/<value>` with namespace and kind non-empty, kind without `/`, value non-empty (it may contain `/`, `[`, `]` and inner whitespace, never leading or trailing whitespace). `rastro:alias` is core, not GitHub: names from later sources contain spaces; the CLI quotes the form and MCP passes JSON. Anything else is an operation error naming the three forms (§18, §29). It is never a JSON-Schema `pattern`, so on MCP it is a result with `isError: true`, not a protocol error. A well-formed form that matches nothing gives the empty answer with coverage. Namespaces are not checked against plugins: the core cannot name `github` (AGENTS.md). Submit uses the same function (`../adr/0008-extractor-role-and-protocol.md`).
    - *Amended 2026-11-05:* the owner decided inner whitespace in the value on 2026-11-05; the first wording was no whitespace anywhere.
35. **Displayed type.** `about` and `why` show the most specific live type when an entity's types are compatible (`vocabulary.md` §15). When they conflict, they show the type with the newest known time, ties broken by claim id. One function; a scoped read computes it from visible justifications. No list of types and no contract change; the conflict stays visible on the type claims.
36. **Lookup does not check alias kinds against installed vocabularies.** Submit already checks kinds; a lookup check would break stored aliases after an extractor is removed.
