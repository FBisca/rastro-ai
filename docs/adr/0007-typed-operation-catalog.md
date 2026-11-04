---
status: accepted
date: 2026-11-04
---

# One typed operation catalog generates the CLI, the MCP tools and the schema

A new library crate, `rastro-operations`, holds the catalog of operations. Each operation has a name, a description, defaults and the surfaces that expose it (`../GLOSSARY.md`: Operation). Request types derive `serde`, `schemars` and `clap`. Result types derive `serde` and `schemars`. CLI parsing and the MCP tool list are generated from the catalog, so the two can't drift (`../decisions/surfaces.md` §2). Human output stays handwritten. The owner chose this on 2026-11-04.

## Schema file
- The generated `schema/operations-v2.schema.json` covers inputs, results and the error envelope, and is committed. A test regenerates it from the catalog and fails on any difference.
- `schema/operations-v1.schema.json` stays, frozen, as it was. Its `$id` and `const 1` must not lie about a changed result.
- Every `--json` envelope moves to `schema_version: 2`. The only consumers today are this repo's tests.
- MCP uses the same result envelope as `--json`, so one request gives equal results on both surfaces.

## Considered options
- **A schema file with Rust types generated from it:** the schema was output-only and couldn't generate inputs. Rust types stay the source of truth.
- **Clap kept separate from the MCP tool definitions:** names, descriptions and defaults would be written three times, as they are today.
- **Regenerating v1 in place:** it would change what v1 promises.

## Consequences
- Adding an operation means adding one catalog entry and its types; exposure on a surface is a field on the entry.
- A schema change is a visible diff in a committed file, reviewed with the code.
- Which operations a surface exposes is catalog data. Slice 2 exposes `search` and `coverage` on MCP; the rest stay CLI-only.
