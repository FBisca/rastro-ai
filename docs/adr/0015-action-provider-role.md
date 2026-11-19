---
status: accepted
date: 2026-11-19
---

# Action providers: a plugin role that declares actions and never reads evidence

Rastro's first write to an outside system needs a home that is neither the core nor a connector. A connector reads untrusted content, and `0004-plugin-manifest-and-grants.md` already has it declare `actions = []`: a plugin that reads what strangers wrote must not also be able to write. The action provider is the other half of that rule, a role on the same supervision and JSON-RPC stdio protocol as connectors and extractors (`0001-rust-core-out-of-process-plugins.md`, `0003-json-rpc-stdio-plugin-protocol.md`). Product meaning: `../decisions/plugins.md` §2, §6 and `../decisions/levels.md` §4.

## Decision
- **Kind.** `action_provider`. A provider is that kind and no other: a manifest that also names `connector`, `extractor` or any other kind is refused, so no plugin reads evidence and holds an action.
- **No evidence permission.** A provider's `permissions.submit` is empty. The host refuses one that lists a kind, naming the plugin and the `submit` permission.
- **Declared actions.** The manifest declares each action as an `[[action]]` table with its `name`, `reach` and `reversibility`, and `permissions.actions` lists the same names. The host refuses a provider whose action lacks either risk, naming the plugin, the action and the missing field, and one whose declaration and permission differ.
  - `reach` is `only_me`, `my_things`, `others_see_it` or `public`.
  - `reversibility` is `can_be_undone`, `can_be_compensated` or `cannot_be_undone`.
  - These are the glossary's words, in the order of `levels.md` §4.
- **Every other plugin.** A manifest of any other kind that declares `permissions.actions` or an `[[action]]` is refused, as a connector with actions already is.
- **Offered actions.** The host offers exactly the actions the installed providers declare and that passed those checks. A refused provider offers none, and the owner reads why on stderr. An action nobody offers is refused naming it.
- **Role at `initialize`.** `role: "action_provider"`, with the protocol version and the manifest id. The provider holds no run binding; each message carries what it needs.
- **Messages (core to provider, requests).** Each carries an `ActionCall`: the `action`, the run's random `key` and the `fields` written for the outside system, and nothing else. Each answers an `ActionResult`: whether the run's thing exists, and its `identifier` and `url`.
  - `action.execute` does the action once for that key.
  - `action.lookup` reports what the key already did, without doing it.
  - `action.undo` reverses it, for an action that can be undone.
- **Environment.** A provider gets its credentials only from its own `permissions.env`, like any plugin (`0004`): a key for an outside system reaches that system's provider and nothing else.

## Considered options
- **Actions in the connector.** One plugin per system, but a connector reads untrusted text, so its content could steer a write. Rejected for the reason above.
- **Actions in the core.** No new process, but the core would own every outside system's client and credentials, against `0001`.
- **Names alone, risk elsewhere.** The host would offer an action it can't classify, and the level rules (`levels.md` §4–§6) have nothing to cap. Declaring risk in the manifest makes a provider without it unloadable.
- **A separate manifest file for actions.** Two files to keep in step; the `[[action]]` tables sit beside the permission that grants them.

## Consequences
- A manifest declares `[[action]]` only for a provider; it is optional and absent means none, so existing manifests stay valid (as in `0008`).
- Granting an action to a source, levels and rules wait for slice 8b; this ADR fixes what a provider declares and what the host accepts.
- The first provider, for Linear, is its own plugin with its own credential grant.
