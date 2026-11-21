---
status: accepted
date: 2026-11-21
---

# Plugin-declared shapes are JSON Schema, checked at runtime by the `jsonschema` crate

`../decisions/plugins.md` §53 has each action provider declare the input and output of its actions. Those shapes arrive when a plugin loads, not when the core compiles, so no Rust type exists for them: `serde` and `schemars` (Rust type → schema, `0007-typed-operation-catalog.md`) cannot check them. The task template's TOML fields (`string` and `integer`, checked by gate code in `crates/rastro-core/src/host/task.rs`) cannot express nested objects, arrays or booleans. The direction must be schema → check, at runtime. Amends `0015-action-provider-role.md`: an `ActionCall`'s `fields` and an `ActionResult` follow the declared schemas.

## Decision
- **Format.** Each `[[action]]` declares an `input` and an `output` JSON Schema, draft 2020-12.
- **Validator.** The `jsonschema` crate, built with `default-features = false`, becomes a runtime dependency of the core. Today it is a dev-dependency of `rastro-cli` only.
- **Required.** Every `[[action]]` declares both `input` and `output`. The host refuses a provider whose action lacks either, naming the plugin, the action and the missing field. The bundled `rastro-action-linear` manifest gains its schemas in the same change; no provider outside the repository exists yet, so the break costs nothing today.
- **Closed declaration.** `ActionDeclaration` refuses unknown keys (`deny_unknown_fields`), so a core that does not know a declared field refuses the provider instead of running its actions unchecked.
- **At plugin load.** The host compiles each declared schema once. A schema that is not valid draft 2020-12, or that breaks a limit below, refuses the provider, naming the plugin, the action and the reason. A refused provider offers no action, as in `0015`.
- **Limits on a plugin schema** (it is untrusted):
  - Only local `$ref`. Without default features the crate fetches no remote reference; a schema that needs one is refused.
  - Every object is closed (`additionalProperties: false`), the rule `close_objects` already applies to the operation catalog.
- **At propose.** The core checks the input before it records anything. Each violation is reported with its JSON path, and nothing is recorded.
- **After `action.execute`.** The core checks the provider's output before it keeps it. An output that does not validate is a provider failure, handled like any other provider error.
- **CLI input.** Top-level `string`, `integer` and `boolean` properties become flags; any other input goes through `--input '<json>'`.
- **Core-held fields.** The action name, `why` and cited claims stay fixed in the core, are never part of a plugin schema and are never sent to the provider (`../decisions/plugins.md` §37).

## Considered options
- **Extend the task template's TOML fields.** Small and consistent with templates, but Rastro would own a schema language and grow it one type at a time. Rejected.
- **Rust types per provider.** Needs the provider's types at compile time, against `0001-rust-core-out-of-process-plugins.md`.
- **No validation in the core; trust the provider.** A provider's output would reach Rastro's records unchecked, and a bad proposal would fail only at the outside system. Rejected.

## Consequences
- Task templates can move to JSON Schema later, so plugin-declared shapes have one validator. Not decided here.
- `schema/plugin-protocol-v1.schema.json` (plugin kit, PR #251) publishes `ActionDeclaration` without `input` and `output`; the change that builds this ADR updates that schema with them.
- The rule match, the duplicate check and the queue edit read Linear fields today; their move to declared fields needs a plan (`../decisions/plugins.md` §53).
