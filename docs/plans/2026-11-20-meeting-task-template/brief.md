# Meeting capture as the first task template plugin

Lane: regular, complex — this changes how a meeting task is defined without changing who can run it or accept its output.

Today the meeting prompt and checks live in core. This plan gives them to a bundled template that owners can replace locally. A generic runner enforces the declared rules without knowing what a meeting is. Three tickets; no new setup for existing imports.

## Your calls

None.

## What changes

### Owner

- **Changed:** Existing imports use the bundled meeting template and produce the same hypothesized commitments on the CLI and through shared MCP reads. (S4)
- **New:** Owners can select a compatible local template without switching adapters. (S5)
- **New:** A missing or incompatible template stops the import instead of silently substituting the default. (S6)
- **Changed:** Replay uses the recorded rules even after the original template changes or is removed. (S7)
- **Changed:** Older recordings replay with bundled legacy rules regardless of the current selection. (S8)
- **Changed:** Forgetting a meeting also removes the saved replay contract. (S9)

### Validation

- **New:** Different domains can use the same rules runner. (S1)
- **New:** If the adapter cannot enforce a required control, it gets no transcript. (S2)
- **Changed:** An excluded candidate is counted without asking the model to try again. (S3)

## Walkthrough

1. Import an exported transcript with `rastro meeting import`. Without an override, Rastro picks the bundled template. (S4)
2. Review the validated hypotheses in `rastro queue`. A candidate matching an exclusion does not cause a retry. (S4, S3)
3. Update or uninstall the template. `rastro meeting replay` still uses the rules that applied to the saved output. (S7)

What can go wrong: an invalid replacement stops import rather than switching templates (S6); an adapter missing a required control receives no transcript (S2).

## Out of scope

Recording or fetching meetings, a second shipped task, a general plugin installer, and changes to the review flow. The [MCP catalog](../2026-11-20-mcp-deferred-catalog/brief.md) is independent; the meeting template adds no MCP tool.

## Risks

- **Enforcement drift:** keep the non-negotiable boundary in core. Refuse unsupported controls before sending a transcript to the adapter.
- **Replay drift:** use the rules saved with the original run, not the currently selected template.

---

# Reference

## Build order

T1 can start once adapter enforcement is on the base branch. T2 integrates with meetings only after the [judgment-adapters meeting scenarios](../2026-11-20-judgment-adapters/plan.yaml) land. Keep the pending MCP removal separate.

- T1 Generic runner for declarative rules (regular · complex).
- T2 Bundled meeting template with stable replay (regular · complex, after T1).
- T3 Explicit local replacement (regular · complex, after T2).

## Decision log

**D1 Meetings first.** Rejected: waiting for a decision task to design the first template contract. Why: move the existing meeting judgment to the plugin boundary before adding another task. Source: user request; [Plugin model §40](../../decisions/plugins.md).

**D2 Bundle the default; allow explicit local replacement.** Rejected: requiring new setup for existing imports or silently falling back when the selected replacement cannot load. Why: preserve the default workflow and honor explicit owner selection. Source: user agreement on Q2.

**D3 Domain-blind task core.** Rejected: moving the prompt out while keeping meeting fields, exclusions, or template-ID dispatch in the core task runner. Why: the same runner must be usable for a meeting, calendar, or blog task without a core change. Core interprets generic checks, owns the trust boundary, and controls retries and recording; the plugin declares the rule logic and dispositions. Source: user acceptance of Q1 with this condition; [Plugin model §40](../../decisions/plugins.md).

**D4 Replay uses the original contract.** Rejected: replaying through the currently selected template or requiring the original package to remain installed. Why: template changes must not change the interpretation of recorded output on replay. Save the validation contract with each run; use the bundled legacy rules for pre-template runs. Replay does not call a model. Source: user agreement on Q3.

## Code touched

`crates/rastro-core/src/meeting.rs` embeds the instructions, candidate fields and checks today. `crates/rastro-core/src/host/judge.rs` owns the adapter seam. `crates/rastro-plugin-sdk/src/lib.rs` defines manifest and enforcement declarations. `docs/specs/meetings.md` describes the current behavior; the judgment-adapters plan also targets it.

## Interfaces

A local declarative package supplies instructions, input and output schema, rules with failure or exclusion dispositions, and the boundary profile. Use generic primitives for schema, text spans, and field comparisons; leave meeting-specific preparation and mapping outside the runner. Compatible replacements honor the meeting integration contract and do not change how the core trust floor is enforced. Persist the validation contract, not a second transcript. The typed operation catalog and adapter wire format stay unchanged in this plan.
