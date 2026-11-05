---
status: accepted
date: 2026-11-04
---

# Extractor role: deterministic, out of process, validated by the core

Turning evidence into claims runs in a minimal **Extractor**, a separate process on the same supervision and JSON-RPC stdio protocol as connectors (`0001-rust-core-out-of-process-plugins.md`, `0003-json-rpc-stdio-plugin-protocol.md`). Connector and Extractor stay separate kinds (`../decisions/plugins.md` §3). The core decides what becomes a claim (`../decisions/plugins.md` §1). Only what slice 3 needs is generalised: deterministic extractors, `observed` claims.

## Shape (protocol stays 1.x, additive)
1. **Role at `initialize`.** The core names the role (`connector` or `extractor`) and the run binding: run, extractor version, and the (source, scope) pairs it may read. An `initialize` without a role means connector, so existing connectors work unchanged.
2. **Core → extractor `evidence.batch` (notification):** a bounded, ordered batch. Each item: evidence id, source, scope, location, field, span (field, byte range, quote), `captured_at`, `source_updated_at` and trust class. Order is (`captured_at`, evidence id). Evidence carries no kind (the store has no kind column); an extractor derives kinds itself, by an extraction rule. The core marks the last batch (`last: true`); the extractor then sends `run.finish`.
3. **Extractor → core `claim.submit` (notification):**
   - `subject`: canonical identifier and type;
   - `predicate`;
   - `object`: an entity (canonical identifier and type) or a literal with a datatype;
   - `extraction_rule`: the id of an extraction rule the manifest declares;
   - `supports`: a non-empty list of evidence id plus span.
4. **Extractor → core `run.finish` (request):** the extractor's status and how many claims it submitted. The core answers with its own counts of submitted, accepted and rejected. The core's counts are the authority.

## Failure rules
Those of `0003`, plus:
- An unfinished extractor run fails as `interrupted` and creates no source coverage window: coverage means what was looked at in the source.
- If extraction fails after ingestion, `sync` exits non-zero, the source's coverage stays as read, its extraction state reads `failed`, and reads serve the last good projection. The next sync resumes from the checkpoint (`0009-claims-projection.md`).

## Core validation
A claim is recorded as `observed` only if all hold; otherwise it is rejected and counted, and nothing of it is stored:
- every support's evidence id was delivered to this run;
- the evidence has trust class `structured`;
- each support's span lies inside that evidence's span;
- the extraction rule is one the manifest declares deterministic;
- the predicate exists in a vocabulary the manifest depends on, and the subject and object types satisfy its domain and range.

Hypothesis claims are not accepted until slice 5. Nothing is ever taken from a title or body.

## Manifest additions
Extends `0004-plugin-manifest-and-grants.md`. All are optional; absent means empty, so existing connector manifests stay valid unchanged.
- `kinds` may include `extractor` and `vocabulary`; one package may provide several kinds;
- `extraction_rules`: the extractor's rule ids, each declared deterministic or not;
- `vocabulary`: the path of the file defining the namespace's types and predicates;
- `dependencies`: the vocabularies the extractor uses;
- `default_extractor` (on a connector): the extractor id bound by default to its sources.

## Binding
An extractor is bound to sources by config, never discovered. A connector's manifest names its `default_extractor`, and each source's config binds it, for new and existing sources alike (GitHub: `rastro-extractor-github`, without the owner editing config). The extractor receives only the evidence of its bound sources. An installed but unbound extractor never runs; a bound but missing one reads `not_installed`.

## Considered options
- **A generic plugin host for every kind:** more than this slice needs; slice 5 reuses the role instead.
- **A declarative mapping run in the core:** no process, but it grows a rules language the core must own.
- **The connector emitting claims:** mixes reading and interpreting, which have different trust (`../decisions/plugins.md` §3).
- **An in-core mapper:** fast to build, but not swappable by replay.
- **Discovering and running every installed extractor:** reads sources the owner never bound.

## Consequences
- Slice 5 reuses the role for agent-tool extractors; its hypotheses need a new, separate acceptance rule.
- The projection that stores accepted claims: `0009-claims-projection.md`.

## Amended 2026-11-05
Status stays accepted. Knowledge graph deep dive (`../research/kg-deep-dive/42-proposed-decisions.md`); product meaning in `../decisions/knowledge.md` §33–§48 and `../decisions/vocabulary.md` §14–§19. Tables: `0009-claims-projection.md`.
- **Justification, not support (amends *Core validation*).** A submission's `supports` are one derivation, an AND bundle: every listed span must satisfy the checks, and a failure of any rejects the claim. Justification identity includes the claim, the extractor and the derivation. Several claims from one span (for example a repository and a kind from one URL) don't collide.
- **Justifications of one claim are OR alternatives.** Inference (later) adds premise claims to a derivation, also AND; cycles are rejected over the whole dependency graph.
- **Location context (amends *Shape*).** For a changed record, the core may deliver companion fields of the same source, scope and location, with evidence no newer than the batch high-water mark; the choice is recomputable on replay, and the delivered ids are kept. Delivery stays bounded per batch.
- **Meta-predicates.** The core owns the `rastro:` prefix. Types and aliases are `rastro:type` and `rastro:alias` claims (the datatype `rastro:type` too). A plugin can't define `rastro:` names. The core's identity predicates (`rastro:held_by`, `rastro:same_as`) are named in the slice 4 ADR.
- **Type grounding.** A submission's claims are validated together. Endpoint types come from type claims in the same bundle, not from stored state; a bundle carries the types of its own endpoints. A type from another bundle that disagrees is a visible conflict, subject to `supertype` compatibility.
- **Vocabulary format (amends *Manifest additions*).** The vocabulary file is parsed with unknown fields denied. `cardinality` is required (`one` or `many`, report-only). `supertype` is optional. Datatypes keep lexical identity.
- **Retirement rule (amends *Binding*).** One binding per source stays. A version bump replays the pair; when the replay completes, the old version's justifications are replaced in the same transaction as the final batch. Reads serve the old output until then, and a failed upgrade keeps the last good output. A slot or replacement contract for several bound extractors waits for a second one.
- **Domain inference is not in the core** (no rule registry). For slice 5, derivation is an extractor role that submits claims with declared premises, which the core validates. That needs a protocol message first.
- **Rebuild source.** The source comes from the `sources` row; nothing is hard-coded to GitHub.
- *Build notes, 2026-11-05 (from code review):*
  - **Grounding unit.** The submission that grounds endpoint types is the whole extractor run: claims are validated together at `run.finish`. Earlier runs never ground a type.
  - **Location context.** The core delivers every latest `structured` field of each changed location, up to the batch high-water mark. It's capped at 64 fields per location, and a location over the cap fails the run (`context_too_large`) rather than being trimmed, so incremental, `--reread` and rebuild agree. The delivered set isn't stored: it is recomputed on replay from evidence and the checkpoint boundary. Only the ids cited in spans are stored.
  - **Identifier trust.** The core checks that every span is structured, delivered and quoted exactly. It doesn't check that a canonical identifier such as `github:account/<id>` equals the quoted `/user/id`; that tie belongs to the extractor's declared deterministic rule.
  - **Type scalarity.** Every `rastro:type` claim is scalar, and carries its type's supertype closure (`type_ancestors`, outside every hash). The lifecycle suppresses `conflicts_with` only between a type and one of its ancestors.

## Amended 2026-11-05 (claims hardening)
Status stays accepted. Product meaning in `../decisions/surfaces.md` §34 and `../decisions/vocabulary.md` §24.
- **Identifier grammar at submit.** `claim.submit` rejects a subject or object identifier outside the entity-form grammar of `surfaces.md` §34, using the same function as the read side; the claim is counted rejected. Otherwise an extractor could mint an entity that `about` cannot reach.
- **Alias checks.** Vocabulary load rejects an alias kind containing `/`, `:` or whitespace. Submit rejects an alias claim whose value is empty or whitespace-only. Lookup doesn't check kinds against installed vocabularies.
