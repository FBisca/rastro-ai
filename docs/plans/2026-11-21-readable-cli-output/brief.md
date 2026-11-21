 Readable CLI output
Lane: regular × complex → Plan. Every command's human output changes style; the design is open and the CLI is a public surface.

Rastro's terminal output is hard to read: every command packs its fields into one pipe-delimited line that overflows the terminal, timestamps come in four formats, and empty results speak in 22 different voices. This plan gives the human output one readable pattern across all 46 commands: a record is a block, one labelled field per line, one timestamp format, one voice for nothing-happened. The machine contract — `--json`, exit codes, stderr, queue behaviour — does not move. Five sequential tickets in this repository, all agent-built; no person steps.

## Your calls
None.

## What changes

### The owner at the terminal
- **New:** every record prints as a block — quote or header first, then one labelled field per line, no pipe-packed lines (S1, S2).
- **New:** a queue item's id stands on its own line, copyable straight into an answer (S3).
- **Changed:** every human timestamp reads `2026-11-21 14:22 UTC` — one format, minute precision (S4).
- **Changed:** empty reads say "No …" in one voice; writes that changed nothing end "; nothing recorded" (S6, S7).

### Scripts and agents
- **Changed:** nothing. `--json` envelopes keep full RFC 3339 times, stay schema-valid and MCP-equal; exit codes and stderr untouched (S5).

### The system's rules
- **New:** three requirements in the surfaces spec pin the block layout, the timestamp format and the empty/no-op voice — they bind every future command too; the style decisions land in the surfaces decision file, and the render module's docs are the pattern guide for implementers.

## Walkthrough
1. The owner runs `rastro search budget` on a store with two matching issues; each match prints its quote verbatim on one line, then its citation fields one per indented line, trust class beside the quote (S1).
2. Captured reads `2026-11-21 14:22 UTC`, not `2026-11-21T14:22:03.490Z` (S4).
3. The two matches are separated by a blank line (S2).
4. The same read with `--json` returns the identical v2 envelope it returns today, RFC 3339 times included (S5).
5. On an empty store, `rastro queue`, `rastro coverage` and `rastro search` each answer with one "No …" sentence (S6); undoing a run with nothing to undo ends "; nothing recorded" (S7).
6. In the queue, the owner copies an id from its own line into `rastro queue accept <id>`; the answer names the same id byte-for-byte (S3).

What can go wrong:
- Instruction-like source text stays escaped and verbatim with its trust class — the injection fixtures stay green (S1).
- Piped output is identical to the terminal; there is no TTY branch to diverge (S5).
- `--json`/MCP queue reads still record no shown report, so automation cannot make an item look ignored (S3).
- Long quotes still clip at 500 characters with `…` unless `--full`; a commitments list with no hypotheses still names every source's gap (existing specs, unchanged).

## Out of scope
- Colour, bold, TTY detection — plain text only; a later plan if wanted.
- Verbosity flags, pagination, `--format`, YAML/CSV — `--json` stays the only machine path.
- Help text — derived from the catalog and spec-pinned.
- `meeting export`'s JSON and MCP output — data payloads, not displays.
- The queue's explanatory preamble — product wording, unchanged.

## Risks
- **Scripts parsing human text:** the restyle changes every human line. Limit: `--json` is the documented machine path and stays byte-equal; exit codes unchanged.
- **Queue state drift:** a restyle could drop or double-count shown reports. Limit: T3 carries the state risk; the existing shown-report tests and S3 run in it.
- **Test churn:** 551 spec-tagged tests assert human fragments. Limit: glossary-word assertions survive; each ticket updates only its own family, red → green.
- **Rollback:** five sequential PRs; reverting one restores the previous style for that family.

---

# Reference

## Build order
- T1 render module (block builder, timestamp, empty voice) with its docs as the pattern guide, coverage/source/sync/check/tick restyled; decisions recorded (regular · complex).
- T2 search/about/why/claims reads restyled (regular · complex, after T1).
- T3 queue/focus restyled, shown reports intact (regular · complex, after T2).
- T4 actions/trust/rules restyled, no-op voice (regular · complex, after T3).
- T5 identity/corrections/meetings/commitments restyled (regular · complex, after T4).

All tickets touch `main.rs` and `render.rs`, so they merge strictly in order; each branches from main after the previous merges.

## Decision log
**D1 Record blocks everywhere.** Rejected: aligned tables for lists; wrapped pipe lines. Why: result sets are small (≤20 items) and fields are long prose, no new dependency or width logic, and piped output stays identical so existing tests keep working. Source: user, grill Q1 2026-11-21.
**D2 No colour, no TTY detection.** Rejected: TTY-gated ANSI with a NO_COLOR convention. Why: no colour dependency exists, all tests run piped, and layout carries the readability. Source: user, grill Q2 2026-11-21.
**D3 One human timestamp format: `YYYY-MM-DD HH:MM UTC`.** Rejected: local time; relative times; keeping the four formats. Why: explicit zone and minute precision read fast; `--json` keeps full RFC 3339. Source: user, grill Q3 2026-11-21.
**D4 One empty/no-op voice.** Rejected: the 22 bespoke strings. Why: "No …" for empty reads and "; nothing recorded" for no-op writes remove the voice drift; glossary words stay exact. Source: user, grill Q4 2026-11-21.
**D5 All 46 commands in this plan, batched.** Rejected: reads-first subset. Why: the pattern only pays off everywhere; writes are the minority of commands. Source: user, grill Q5 2026-11-21.
**D6 Machine contract and queue behaviour untouched.** Rejected: restyling `--json`, stderr or exit codes alongside. Why: pinned by the surfaces spec and decisions D9/D44/D45 and the queue spec; a change there is a separate plan. Source: user, grill Q6 2026-11-21; docs/specs/surfaces.md, docs/specs/queue.md.
**D7 No verbosity, pagination, `--format` or help rewrite.** Rejected: `-v`/`-q`, `--format human|json`. Why: `--json` is the settled machine path (surfaces D9) and `--full`/`--limit` already bound output. Source: user, grill Q7 2026-11-21; docs/decisions/surfaces.md D9.

## Code touched
- `crates/rastro-cli/src/main.rs` — the whole human rendering: one `human(&Response)` match, ~41 ad-hoc helpers, 1578 lines. Modified by every ticket.
- `crates/rastro-cli/src/render.rs` — new shared render module (block builder, timestamp format, empty/no-op voice), created by T1.
- `crates/rastro-cli/src/check.rs`, `crates/rastro-cli/src/tick.rs` — their own printing, bypass the `human()` path. Modified by T1.
- `crates/rastro-cli/tests/` — 63 files, 551 `spec:` tags; subprocess runs of the real binary with piped stdout, `stdout.contains(...)` for human text, `serde_json` for `--json` (164 references). The seam new rendering tests reuse; no snapshot crate exists.
- `crates/rastro-core/src/lib.rs` — `display_safe` (escape + clip at 500 chars with `…`) and `display_escaped`: the only output-safety primitives; every source string keeps routing through them. Unchanged.
- `crates/rastro-operations/src/envelope.rs`, `schema/operations-v2.schema.json` — the `--json` v2 envelope. Unchanged.
- `docs/decisions/surfaces.md` — T1 appends the style decisions with the date.
- Spec: `docs/specs/surfaces.md` — three requirements added on archive.

Settled contracts the gather confirmed (sources: `docs/specs/surfaces.md`, `docs/decisions/surfaces.md` D9/D44/D45/D46/D12/D13/D17/D53, `docs/decisions/renderers.md` D9, `docs/specs/queue.md`):
- Global boolean `--json`; one v2 envelope validating against the committed schema; CLI `--json` and MCP envelopes byte-equal for search/coverage/about/why/commitments.
- Results on stdout; progress, warnings (`warning: <text>`) and errors (`error: …`) on stderr; broken stderr loses lines without panic or exit-code change.
- Exit codes: 0 success, 1 operation failure, 2 usage error.
- Source text escaped and clipped via `display_safe`/`display_escaped`; trust class beside evidence; results labelled, never rewritten.
- Terminal queue read records one shown report per printed item; `--json`/MCP reads record none. Queue answers name the content-addressed item id exactly as printed.
- Glossary terms are the output vocabulary (Coverage states, Status, trust classes), shared with JSON via `state.as_str()`.
- The hard rule (nothing about Rastro exposed to a source) binds outbound writes, not terminal output to the owner.

## Interfaces
Measured current output (scratch store, scripted test connector, two evidence bodies; captured 2026-11-21):

```
$ rastro search budget
"Alice will send Bob the budget by Friday. The numbers need a final review before the board sees them."
  Link: o/p#12 | Field: /body | Span: [0, 101) | Captured: 2026-11-21T14:22:03.490Z | Trust class: untrusted content | Scope: personal | Source: demo
  Coverage: demo | Scope: personal | Coverage: found | Last checked: 2026-11-21T14:22:03.504Z | Fresh until: 2026-11-21T15:22:03.504Z

$ rastro coverage
demo | Scope: personal | Coverage: found | Last checked: 2026-11-21T14:22:03.504Z | Fresh until: 2026-11-21T15:22:03.504Z
  [2026-08-23T14:22:02.867Z, 2026-11-21T14:22:03.485Z) | found | Selection: all

$ rastro source list
demo | Connector: test | selector: {"repo":"o/p"} | Scope: personal | Freshness rule: 1h | window start: 2026-08-23T14:22:02.867226Z

$ rastro focus
Nothing needs you.
Queue: 0 waiting
As of 2026-11-21T14:22:03.553312+00:00
```

The same search in the proposed block style (illustrative — the spec pins structure, not these exact strings; field order and labels are ticket-level):

```
"Alice will send Bob the budget by Friday. The numbers need a final review before the board sees them."
  Trust class: untrusted content
  Source:      demo (personal)
  Link:        o/p#12 (/body, span [0, 101))
  Captured:    2026-11-21 14:22 UTC
  Coverage:    found
  Checked:     2026-11-21 14:22 UTC
  Fresh until: 2026-11-21 15:22 UTC
```

Empty-state voices today: `Nothing is waiting` (queue), `No sources registered` (coverage), `no evidence found for "…"` (search), `Nothing needs you.` (focus) — 22 bespoke variants across commands.

Counts from the gather: 46 CLI-exposed operations (34 CLI-only, 12 CLI+MCP), 41 human render helpers, 22 empty/no-op strings, 4 timestamp formats, 0 colour/TTY/table dependencies, search default limit 20 (max 200), about/why max 50, coverage shows 10 windows.