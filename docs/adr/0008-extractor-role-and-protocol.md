---
status: accepted
date: 2026-11-04
---

# Extractor role: deterministic, out of process, validated by the core

Turning evidence into claims runs in a minimal **Extractor**, a separate process on the same supervision and JSON-RPC stdio protocol as connectors (`0001-rust-core-out-of-process-plugins.md`, `0003-json-rpc-stdio-plugin-protocol.md`). Connector and Extractor stay separate kinds (`../decisions/plugins.md` §3). The core decides what becomes a claim (`../decisions/plugins.md` §1). Only what slice 3 needs is generalised: deterministic extractors, `observed` claims.

## Shape (protocol stays 1.x, additive)
1. **Role at `initialize`.** The core names the role (`connector` or `extractor`) and the run binding: run, extractor version, and the (source, scope) pairs it may read. A plugin that doesn't know the role field is a connector, so existing connectors work unchanged.
2. **Core → extractor `evidence.batch` (notification):** a bounded, ordered batch. Each item: evidence id, source, scope, location, field, span (field, byte range, quote), `captured_at`, `source_updated_at` and trust class. Order is (`captured_at`, evidence id). Evidence has no kind (`0005-evidence-observations-and-spans.md`); an extractor derives kinds itself, by a declared rule.
3. **Extractor → core `claim.submit` (notification):**
   - `subject`: canonical identifier and type;
   - `predicate`;
   - `object`: an entity (canonical identifier and type) or a literal with a datatype;
   - `rule`: the id of a rule the manifest declares;
   - `supports`: a non-empty list of evidence id plus span.
4. **Extractor → core `run.finish` (request):** the extractor's status and how many claims it submitted. The core answers with its own counts of accepted and rejected. The core's counts are the authority.

Failure rules are those of `0003`. An unfinished extractor run fails as `interrupted` without any source coverage window, because coverage means what was looked at in the source.

## Core validation
A claim is recorded as `observed` only if all hold; otherwise it is rejected and counted, and nothing of it is stored:
- every support's evidence id was delivered to this run;
- the evidence has trust class `structured`;
- each support's span lies inside that evidence's span;
- the rule is one the manifest declares deterministic;
- the predicate exists in a vocabulary the manifest depends on, and the subject and object types satisfy its domain and range.

Hypothesis claims are not accepted until slice 5. Nothing is ever taken from a title or body.

## Manifest additions
Extends `0004-plugin-manifest-and-grants.md` (a package may provide several kinds):
- `kinds` may include `extractor` and `vocabulary`;
- `rules`: the extractor's rule ids, each declared deterministic or not;
- `vocabulary`: the path of the file defining the namespace's types and predicates;
- `dependencies`: the vocabularies the extractor uses.

## Binding
An extractor is bound to sources by config, never discovered. Each connector names a default extractor, applied to new and existing sources of that connector (GitHub: `rastro-extractor-github`). The extractor receives only the evidence of its bound sources. An installed but unbound extractor never runs; a bound but missing one reads `not_installed`.

## Considered options
- **A declarative mapping run in the core:** no process, but it grows a rules language the core must own.
- **The connector emitting claims:** mixes reading and interpreting, which have different trust (`../decisions/plugins.md` §3).
- **An in-core mapper:** fast to build, but not swappable by replay.
- **Discovering and running every installed extractor:** reads sources the owner never bound.

## Consequences
- Slice 5 reuses the role for agent-tool extractors; its hypotheses need a new, separate rule.
- The projection that stores accepted claims: `0009-claims-projection.md`.
