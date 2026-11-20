# Brief: Judgment adapters
Slug: `judgment-adapters`. Contract: `plan.yaml`. Lane: Plan, regular/complex: new adapter crates plus a session protocol touching trust boundaries and credentials (sensitive).

## Why
Judgment is one-shot today: one bad quote fails a whole meeting import, and only Claude Code can judge. PR #205 approved capped retry sessions, thin drivers for any harness, and a System-1 decision class. This plan builds the drivers and the session protocol, in parallel with credential-checks where the paths allow.

## What Changes

### The owner
- **New:** a codex adapter and a pi adapter. Bind either through `[commitment_adapter]` for one-shot meeting judgment today. Tickets T3, T4; crates `crates/rastro-adapter-codex/`, `crates/rastro-adapter-pi/` (S9, S10).
- **New:** the claude adapter answers session turns, not just one-shot (S11, S12 · T2 · `crates/rastro-adapter-claude-code/`).
- **New:** a decision adapter crate driving your jev scoring service (the agent-review scorer). Nothing binds it yet; binding and the first decision task wait for a later plan (S13, S14 · T5 · `crates/rastro-adapter-decision/`).
- **Changed:** a failed gate earns a retry turn in the same session instead of failing the import. The cap is 3 turns, 10 minutes per turn. Tickets T7, T9; `crates/rastro-core/` (S1, S2, S3).
- **New:** a run is refused before the adapter starts when its manifest cannot enforce the lockdown floor — tools off, no MCP servers, no setting sources — naming the missing control (S7, S8 · T8 · `crates/rastro-core/`).

### The harness
- **New:** each retry turn resumes the harness session the first turn produced, in a fresh process with the lockdown flags set again (S11 · T2, T3, T4).
- **New:** when the harness cannot resume, the adapter runs a fresh turn carrying the prior answer and the failure reason, and records the degradation (S12 · T2, T3, T4).

### The system's rules
- **New:** every turn is recorded: request, answer and gate verdict, including failed turns (S4 · T6, T7, T8 · `crates/rastro-storage-sqlite/`, `crates/rastro-core/`).
- **New:** only the final validated output becomes evidence (S5 · T7).
- **New:** exports leave turn history out (S6 · T6, T8).
- **Changed:** an invalid quote earns capped retry turns before it fails the run; the meetings spec wording follows (S2, S3 · T9 · `docs/specs/meetings.md`).
- **New:** manifests declare the boundary controls they enforce; the SDK knows the `score_text` capability and the session and decision message types (S9 · T1 · `crates/rastro-plugin-sdk/`).

### Rollout
- **Changed:** ADR-0016 accepted, its stale pi tools-off note corrected (T1 · `docs/adr/0016-judgment-sessions-capped-retry-turns.md`).
- **Changed:** plugins.md §41 says thin CLI drivers by default (T1 · `docs/decisions/plugins.md`).
- **Changed:** plugins.md §45's per-turn timeout default becomes 600s (T1 · `docs/decisions/plugins.md`).
- Two waves: T1, T2, T3, T4, T5, T6 run now on unheld paths; T7, T8, T9 carry files the credential-checks plan holds, and wait for it to merge (D4, D8).

## Capabilities
Modified spec `docs/specs/commitments.md`:
- one requirement restated whole: "The core calls the adapter and keeps only the candidates that pass its checks";
- five added: turn recording; enforcement matching; thin harness drivers; resume or recorded degradation; the decision adapter.

`docs/specs/meetings.md` gets a coordinated reword of its invalid-quote requirement in T9, title unchanged so its test tags stay valid (D13).

## Impact
- SDK: `crates/rastro-plugin-sdk/src/lib.rs` — additive session params on `judgment.request`, optional `session_id`/`degraded` on the answer, an optional `enforcement` manifest field, `score_text` in the capability list, `decision.request`/response types.
- Core (wave 2): `crates/rastro-core/src/host/judge.rs` (session supervision, the check that the manifest declares its controls), `crates/rastro-core/src/meeting.rs` (gates as feedback, caps), `crates/rastro-core/src/lib.rs` (`[commitment_adapter]` gains `max_turns`, timeout default 600s).
- Storage: `crates/rastro-storage-sqlite/src/meeting.rs` — a turn-history table beside `meeting_run`/`meeting_output`.
- Fixtures: the 7 CLI test files that drive judgment with a fake adapter script (66 tests); the fake script gains session branches. Adapter crates test against fake harness scripts; live contract tests run once and record their result (D15).
- New crates edit `Cargo.toml` members (unheld).

## Context
The shipped judgment path is one-shot: the core sends one `judgment.request` to the configured `agent-tool-adapter` and validates the answer itself (ADR-0012). PR #205 (merged 2026-11-20) approved capped multi-turn judgment sessions (ADR-0016, proposed), thin CLI drivers over the shared SDK, and a second adapter class for System-1 decisions. The credential-checks plan holds `crates/rastro-core/` and `crates/rastro-cli/` until it lands; the SDK, the adapter crates, `rastro-storage-sqlite` and `Cargo.toml` are not held by another plan (gather, 2026-11-20). Installed harnesses verified:
- claude 2.1.329: `--resume`;
- codex 0.169.0: `exec resume`, `--output-schema`;
- pi 1.6.0: `--session-id`, `--session-dir`, `--no-tools`, `--no-mcp`.

## Example
Synthetic: a meeting import bound to the claude adapter, turn cap 3.

1. Turn 1: the core sends `judgment.request`; the adapter drives `claude -p` under lockdown and relays candidates. One quote is not a contiguous piece of its turn (S1).
2. The core's quote gate fails. It records the turn and sends the failure reason — gate, field, expected, actual — as turn 2 of the same session (S2, S4).
3. The adapter resumes the claude session id turn 1 produced, with the safety flags set again, in a new process; the corrected answer passes every gate (S11).
4. Only the final validated candidates become evidence; the run's history holds both turns (S5).

Variants:
- Turn 3 still fails the quote gate: cap exhausted, the run fails, last good results stay stale — kept but marked out of date (S3).
- The bound pi adapter has not proven tools-off: the run is refused naming the control, the adapter never starts (S7).
- The harness cannot resume: a fresh turn carries the prior answer and the reason, degradation recorded (S12).
- A `decision.request` reaches the decision adapter: schema-valid scores are kept; a broken shape fails the run, last good results stay stale (S13, S14).
- Export: turn history stays out (S6).
- A manifest declaring every required control lets the run proceed (S8).
- A new adapter's manifest passes the SDK check (S9).
- A one-shot request drives the harness under lockdown (S10).

## Goals / Non-Goals
Goals: codex and pi adapters bindable today; claude session-capable; capped retry sessions with core-executed gates, turn recording and enforcement matching; a decision adapter crate proving the System-1 class on the same protocol.

Non-Goals: task template plugins (§40 stays core); cost-cap enforcement (cost recorded when the harness reports it); decision-adapter binding and the first decision task (needs held core and open question 17); live jev verification (waits for binding); the agent-runtime adapter implementation (optional, parking lot); open questions 16 and 18.

## Decisions
- D1 Thin CLI drivers over the shared SDK (`crates/rastro-plugin-sdk`) are the default adapter shape; agent-runtime is an optional implementation, never a dependency (ADR-0016 §4; plugins.md §36 amended; gather).
- D2 Session defaults: turn cap 3, cost cap owner-set; gate failures retryable in-session, system failures split recoverable/non-recoverable (ADR-0016 §8; plugins.md §45; gather).
- D3 Gates are declarative and core-executed; the adapter never validates its own trust boundary; every turn including failed ones is recorded; only validated output becomes evidence; last good results stay stale on failure (ADR-0016 §2–3; gather).
- D4 New harness adapters are new workspace crates using the SDK. The session loop, gate execution, config binding and all 7 CLI judgment fixture files live in `crates/rastro-core/` and `crates/rastro-cli/`, held by credential-checks T2; tickets touching them wait for it to land. SDK, adapter crates, storage and Cargo.toml are unheld (gather: credential-checks plan.yaml + code seams).
- D5 pi's tools-off flags exist in installed pi 1.6.0 (`--no-tools`, `--no-builtin-tools`, `--tools`, `--exclude-tools`, `--no-mcp`); ADR-0016 §6's "no flag in pi --help" note is stale. The pi adapter declares tools-off enforcement only after a contract test proves it (gather --help; ADR-0016 §6).
- D6 Harness continuation: claude `-p --resume <id>`, codex `exec resume <id>` with `--output-schema`, pi `--session-id`/`--session-dir`; the claude adapter drops `--no-session-persistence` for session-capable runs; boundary flags re-applied on every turn; a fresh process per turn, no process outlives its run (ADR-0016 §4; gather).
- D7 Session state lives under RASTRO_HOME where the harness allows: pi `--session-dir`; claude `CLAUDE_CONFIG_DIR` (already in the shipped manifest's permissions.env); codex's config-dir env var name confirmed by contract test (ADR-0016 §7; gather).
- D8 One plan, two waves: T1–T6 run now on unheld paths; T7–T9 carry the held paths, so the conflict gate blocks them until credential-checks merges. Rejected: two plans — the contract stays whole (owner, round 1 Q1).
- D9 First delivery is the minimal session: today's core checks become the gates, cap 3 turns, per-turn timeout 600s (owner: "turns longer 10m", overrides §45's 120s default; T1 amends §45), turn recording, resume with recorded degradation, a fixed core floor matched against manifest enforcement. Rejected: full ADR-0016 with template plugins and cost enforcement — nothing has designed the template contract yet (owner, round 1 Q2a + 10m).
- D10 Retryable gates: shape violations and invalid quotes. Owner-involvement exclusion stays an exclusion, counted not failed; a stated date outside the quote still reads unknown. Rejected: retrying exclusions — they are not gate failures (owner, round 1 Q4; ADR-0011 §4).
- D11 Decision adapter: same kind `agent-tool-adapter`, new capability `score_text`, new `decision.request` wire; crate plus fake-service contract tests now, binding and first task in a later plan. Target: jev's scoring API, endpoint env-configured. Rejected: full binding now (breaks parallelism, needs open question 17); dropping it (owner, round 1 Q3a).
- D12 ADR-0016 accepted with §6's pi note corrected; plugins.md §41 amended to thin-driver default; §45 timeout default 600s. Rejected: leaving §41 contradicting the owner-approved §36 amendment (owner, round 1 Q5; gather).
- D13 One spec per plan (engine `plan_document.py`): spec is `commitments.md`; the meetings.md invalid-quote requirement is reworded in T9 as a coordinated edit, title kept so test tags stay valid. Rejected: a second tiny plan; retry-free quotes to avoid the edit (engine constraint; linear-ids D5 precedent).
- D14 Wire is additive: `judgment.request` gains an optional session object (turn, feedback, prior answer); the answer gains optional `session_id` and `degraded`; the adapter process lives exactly its run; `decision.request` carries instructions, input and schema, answered with output. Rejected: a new protocol major; an adapter-side session loop (ADR-0003 additive precedent; ADR-0016 §4–5; D3).
- D15 Live contract tests (pi tools-off, claude/codex config-dir relocation, resume round-trips) run once live inside the driver tickets and record their result; CI keeps fake-harness tests. Rejected: fake-only proof of enforcement — the ADR demands a real proof before pi declares tools-off (ADR-0016 §6–7; gather fake-harness pattern).
- D16 Turn history follows recorded-output retention: export omits it, forgetting removes it; granularity assumed full request/answer/verdict per turn. Rejected: dropping failed turns — the ADR records every turn (ADR-0016 §3; open question 16 left open).

## Risks / Trade-offs
- Trust boundary: enforcement matching is the guard; a manifest that under-declares is refused, never degraded (S7). The core floor stays non-negotiable. Live proof before pi declares tools-off (D15).
- Held-crate conflict: T7–T9 cannot start until credential-checks merges; the plan's second wave idles behind it. Mitigation: wave 1 delivers bindable adapters regardless.
- Wrong binding: a `score_text`-only adapter bound via `[commitment_adapter]` passes the manifest check but fails the run cleanly when sent `judgment.request`; capability-to-task matching ships with binding (later plan).
- Session files hold untrusted transcript text under RASTRO_HOME; residual harness stores in the owner's home are disclosed where relocation fails, and slice 14 forgetting covers only what Rastro controls (ADR-0016 §7).
- Live contract tests cost model calls and can flake; they run once, record, and CI stays on fakes (D15).
- Rollback: wave 1 crates are additive; the one-shot path and its specs stand until wave 2 lands, so reverting wave 2 restores today's behaviour.

## Open Questions
- Question 16 (failed-turn granularity, export/forget of harness session stores): assumed full recording with recorded-output retention (D16); the question stays open.
- Question 17 (decision evidence contract): deferred to the binding plan; the crate proves the wire, not the evidence semantics.
- Question 18 (export vs recorded output): untouched.
- codex's config-dir env var name: contract test confirms (D7); `CODEX_HOME` is the candidate.