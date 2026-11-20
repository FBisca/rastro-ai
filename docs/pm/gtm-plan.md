# Rastro: go-to-market plan

Replace one owner's recurring source review with cited understanding and governed actions, then prove the same outcome with a small cohort. Success is what the owner stops checking by hand, with visible gaps, correction, control and a usable exit path ([concept](../concept.md), [glossary](../GLOSSARY.md)).

**Status:** direction approved 2026-11-20; execution starts on PR merge. Pre-alpha; no user database. Future durations, effort, allocations, cohort sizes and numeric gates below are proposals/estimates, not observed results.

## Verified current state

| Area | What works / evidence | Boundary |
|---|---|---|
| Slices 1–7, including 4b | Cited search, coverage, claims, identity, meeting commitments, queue and corrections; [roadmap](../roadmap.md), [claims](../specs/claims.md), [meetings](../specs/meetings.md), [queue](../specs/queue.md), [corrections](../specs/corrections.md). | Roadmap completion records do not independently prove live use. |
| 8a: First action, proposed | Proposed Linear issue, acceptance, creation, inspection, retry, guarded undo and manual tick; merged in [709567f lineage](https://github.com/FBisca/rastro-ai/commit/709567f). | Live switch day unverified; code merged does not mean slice done. |
| 8b: First Act, T1–T4 | Rules/config [a46f9b8](https://github.com/FBisca/rastro-ai/commit/a46f9b8), pending Act [90d240b](https://github.com/FBisca/rastro-ai/commit/90d240b), due execution/undo [3afa889](https://github.com/FBisca/rastro-ai/commit/3afa889), recheck [30e0b0e](https://github.com/FBisca/rastro-ai/commit/30e0b0e). | **8/9 tickets merged** at origin/main `044ee0f`; T9 remains owner work. |
| 8b: T5–T8 | Stop [9e6b44d / #200](https://github.com/FBisca/rastro-ai/commit/9e6b44d), daily cap [4ab9018 / #201](https://github.com/FBisca/rastro-ai/commit/4ab9018), pending reads [fb114a0 / #202](https://github.com/FBisca/rastro-ai/commit/fb114a0), rules check [044ee0f / #203](https://github.com/FBisca/rastro-ai/commit/044ee0f). | Catalog now exposes `rules.check` on CLI + MCP; owner writes remain CLI-only. |
| Checks | Suite at `044ee0f` (2026-11-20): **689 passed, 0 failed, 7 skipped, exit 0**; at `30e0b0e`: 677 passed, exit 0. | Fixture suites only; live providers and stranger journeys unverified. |
| Unverified / gaps | T9 live switch + installed minute scheduler; stranger install, Linux parity and general export/rebuild/privacy proof remain open ([roadmap M5](../roadmap.md#m5-open-and-portable)). | README still says “doesn't act yet” and six MCP tools: G2 remains. Current Claude Code agent tool adapter is distinct from Codex reading over MCP ([plugins §29–36](../decisions/plugins.md)). |

## Wedge

For agent-CLI owners reconciling meetings with GitHub/Linear, test a bounded combination in one daily review: cited evidence and honest coverage, correctable claims, governed actions and portable knowledge; learned trust and general portability remain delivery work. Citations and editable memory already overlap with [NotebookLM citations](https://support.google.com/notebooklm/answer/16164461?hl=en) and [OpenAI memory](https://help.openai.com/en/articles/8590148-memory-in-chatgpt), so the combination is a demand hypothesis without an exclusivity claim. [Dot's shutdown](https://new.computer/) makes continuity a practical lesson: prove usable exit before promising independence.

## Phases and switch days

G-items layer on existing slices; no new slices. Approved order swaps M4/M5 delivery: 8a → 8b → 9 → 13–15 before strangers; 10–12 follow launch. Overall estimate: **~12–16 weeks, ~70–100 engineer-days**; preparation may overlap, acceptance gates remain sequential ([roadmap](../roadmap.md)).

| Phase / estimate | Slices + G-items | Outcome / switch day | Exit gate | Kill criterion |
|---|---|---|---|---|
| **P0 · ~2 weeks** | 8a live + 8b T9; G2, G3 macOS, G7 scoped to current Act reach | Owner's next real issue goes through Rastro; a rule performs the first governed Act instead of manual creation/retry. | Live stop, cap, recheck and guarded undo; installed minute tick observed; 0 safety escapes. Keep the needed issue; test undo on a disposable issue. | Any unauthorized/duplicate write, unsafe undo or silent scheduler failure: disable affected Act; hold gate. |
| **P1 · ~3–4 weeks** | 9; local metrics from `queue.outcomes`/`missed` + owner journal | Owner replaces their chosen daily review; sees trust per cell and usefulness. | Promotion proposed to owner; demotion automatic and visible; local measurement usable. | Unauthorized promotion or no displaced review: stop expansion, narrow workflow. |
| **P2 · ~4–6 weeks** | 13 + 14 + 15; G1, G4, G5, G6, G9; repeat G7 | First cohort installs and reviews without builder help; can leave with usable knowledge. Design 13 after 9 to cover trust natively; decide open question 18 inside 13. | Unassisted install ≤30 min; export/rebuild, scopes, forgetting and plugin lifecycle proved; both adapters tested; baselines measured. | Lost knowledge, forbidden access/egress or failed exit path: hold cohort expansion; install failure: simplify. |
| **P3 · ~4–6 weeks** | Linux parity; G3 Linux, G9, G10; repeat G1/G5/G7 | Same owners replace the review each week; macOS + Linux ready for launch as concept requires. | Sustained ritual; P2-derived numeric gates met; 0 escapes; configured action/interruption budgets respected. | No retained ritual or improving value, any escape/budget breach, or unsupported Linux: hold launch. |
| **P4 · by demand** | G11; M4 rituals 10–12; M6 open judgment; 16 sharing; G8 post-GTM | Owners keep the ritual through upgrades; add surfaces, tasks or sharing only when useful. | Upgrade/restore works; new demand justified; 10/11 → 12 and 13/14 → 16 dependencies preserved. | Recurring low usefulness or excess support/cost: narrow/remove by owner decision; safety escape stops affected Act. |

A safety escape is an executed unauthorized or untrusted-content-driven write, scope/egress breach, cap bypass or unsafe undo. Nothing about Rastro goes to a source; justification and undo stay in Rastro ([AGENTS.md](../../AGENTS.md), [plugins §37](../decisions/plugins.md)).

## Work register

| ID | Work | Phase | Owner role | Exit |
|---|---|---|---|---|
| G1 | Install / quickstart | P2; Linux P3 | Integration | Stranger completes first cited review, correction and setup ≤30 min unassisted; attempts/times recorded. |
| G2 | README truth | P0; each release | Integration | No-action/six-tool claims replaced against exact release catalog; CLI writes and scheduling limits explicit. |
| G3 | Scheduler operations | P0 macOS; P3 Linux | Integration | Installed minute scheduler; inspectable cadence/freshness; overlap, sleep/restart and failure/recovery proved. |
| G4 | Codex agent tool adapter | P2 | Integration | Thin CLI driver + shared SDK; Claude Code/Codex validation, replay, failure, cost and egress proved; agent-runtime optional, never dependency (§36 amended; ADR-0016). |
| G5 | Privacy / exit-path proof | P2 | Core + verifier | Clean rebuild preserves declared semantics including trust; scopes/forgetting/retention and provider swap proved; #18 resolved first. |
| G6 | Plugin lifecycle | P2 | Integration | Install, permission approval/diff, denial, revoke and rollback work; governance boundary documented. |
| G7 | Safety review | P0; changed reach each phase | Independent verifier | Adversarial source/proposal, permission, retry/race, stop/cap/recheck/undo and outbound-payload checks; live proof; 0 unresolved escapes. |
| G8 | Anonymous aggregate export plugin | Post-GTM | Cohort + verifier | Only on demand; inspect-before-send, consent/revoke and anonymity proved; journals suffice at n≤8. |
| G9 | Cohort operations | P2–P3 | Cohort | Authorized accounts/transcripts, fixed roster, local baseline journals, weekly interviews, dropout reasons and same-task host comparison. |
| G10 | Gated launch | P3 | Cohort + lead | Verifier clears release claims, install, Linux, safety, privacy, support and retention; owner accepts launch. |
| G11 | Product upkeep | P4 | Core + cohort | Upgrades/restore, host/connector contracts, usefulness/cost review and support stay within agreed capacity; low-value work narrowed. |

## Metrics without a user database

**North star:** early surfacing share rises while manual checking falls for the owner's chosen review. Compute each owner's review-window median minutes/day ÷ their own baseline median; proposed P3 direction: **median of owner ratios ≤50%**. Set final numeric gates from P2 baselines before P3 observation; acceptance should rise, duplicate/stale surfacing approach 0. Prototype ~30% acceptance and ~half of drops already done/tracked are **not comparable baselines**: denominators/windows unknown ([concept](../concept.md#what-the-prototype-taught-us)).

| Metric | Numerator / denominator | Window | Local collection |
|---|---|---|---|
| Early surfacing + manual minutes | Per owner: actionable commitments surfaced with evidence before manual discovery / union discovered by Rastro, manual review or `missed`; time ratio as above. | P2 baseline; weekly P2–P3 | Owner journal timestamps/minutes + `missed`; same scope, deduplicated commitments, explicit coverage gaps. |
| Activation | Owners completing cited review + coverage + correction + setup ≤30 min unassisted / all invited; failed/nonstarted attempts remain. | Each P2 intake; Linux P3 | Local timed journey log; assisted attempts separate. |
| Ritual retention | Same owners completing replacement review in every observed week / fixed invited roster; dropouts remain. | Weekly P2–P3; P4 follow-up | Owner journal + weekly interview; ticks/downloads are not reviews. |
| Proposal usefulness + quality | Accepted unchanged/edited by cutoff / proposals first shown in exposure window; duplicate/stale items surfaced as outstanding / all items surfaced as outstanding. | Weekly exposures + proposed 7-day maturation lag | Terminal shown reports + `queue.outcomes` + journal; pending remains denominator; late accepts stay separate; edited acceptance is not clean trust. |
| Missed | Owner-validated missed commitments / same actionable union as early surfacing. | Weekly P1–P3 | Local `missed` + journal; no reports does not prove no misses. |
| Control + economics | Escape count; budget breaches / configured periods; provider spend / accepted proposals, with per-kept-run cost separate. | Each run; weekly review | Local action/run reads, owner follow-up and invoices; missing follow-up is unknown, never kept. |

Owners retain journals/outcomes; cohort role reads voluntary summaries, no central profiles or raw evidence telemetry. Report counts, per-owner ratios, missing days and coverage; zero denominators stay undefined and sparse evidence extends gates. Pending is not ignored; unseen items/absence teach nothing; promotion always needs owner intent ([learning §6–10, §16–19](../decisions/learning.md), [queue](../specs/queue.md)).

## Cohort, channels and pricing

Proposed wave 1: **5–6 deep owners, 8 stretch**, macOS first; Claude Code primary + Codex wave. Each has authorized GitHub/Linear access and a permitted transcript.
Direct invitations → gated GitHub release / HN technical demo after P3; compare the same review with the owner's host-native workflow. Equivalent value at lower burden means narrow positioning.
Free Apache-2.0 core; owner-paid provider costs disclosed; telemetry off. Paid sync/hosting waits for willingness-to-pay evidence and a separate decision.

## Delivery crew and gates

Proposed capacity: **2 engineering equivalents + 0.5 cohort/PM + owner 1 h/week**; core owns state/trust/privacy, integration owns install/adapters/scheduling, cohort owns journals/recruitment/support.
Independent verifier did not implement the change and can return it; safety-critical slices require adversarial checks plus live proof at the release commit.
Follow [development checks](../development.md): affected nonempty `cargo nextest run`, required CI gates, spec coverage when requirements/tags change; record commands/results/skips. Switch day completes the slice.
Proposed staged allocation: **first 8 weeks: 16 engineer-weeks + 4 cohort/PM-weeks + US$1,000 external cap**; review actual burn and evidence before further allocation. No paid acquisition; later work not funded by implication.

## Owner decisions recorded · 2026-11-20

Approved lean P0–P4 direction; execution on merge. Local journals replace G8 for GTM; Linux moves to P3; 10–12 move post-launch; baseline-first gates replace invented thresholds. Slice 13 moves to P2 after 9 and before strangers; export covers trust natively and resolves #18 ([open questions](../open-questions.md)).
Approved **A+B adapters**: thin harness CLI drivers + shared SDK; agent-runtime optional, never an adoption dependency ([plugins §36 amended](../decisions/plugins.md)).
Task/action primitives remain end goal: internal tasks vs external actions, deterministic/agent/decision subtypes, task-template plugins, agent/decision adapters and core-executed gates with capped retry turns; M6 starts with duplicate/stale triage + categorization by demand ([plugins §38–43](../decisions/plugins.md)). **[ADR-0016](../adr/0016-judgment-sessions-capped-retry-turns.md) accepted**; shipped one-shot judgment unchanged.
Decided 2026-11-20: #14 no task write exceptions, #15 retry defaults (cap 3, 120s/turn, owner-set cost cap), #9 remove = config only, #4 no central registry at GTM, #11 owner-approved namespaces ([plugins §44–47](../decisions/plugins.md), [knowledge §76](../decisions/knowledge.md)). Remaining #16–18: failed-turn/session recording and export/forgetting, decision-output evidence contract, recorded-output/export tension. Older opens (learning, focus, sharing) remain with their owning slices ([open questions](../open-questions.md)).

## TODO register

| Unknown | Owner role | Closing evidence / gate |
|---|---|---|
| Live proofs | Owner + core/integration + verifier | 8a switch, 8b T9 governed Act and installed minute scheduler; privacy/rebuild and Linux proof at their phases. |
| Baselines / retention / demand | Cohort | Per-owner P2 journals, activation/support attempts, mature outcomes, missingness and same-task comparisons; fix P3 gates before observation. |
| Costs / capacity | Lead + owner | Staffing availability, actual engineering effort, labor/provider costs and burn against staged allocation; revise estimates at gates. |
| Willingness to pay | Cohort + owner | Priced concept test before paid-service scope; no revenue forecast. |
| Open questions | Owner + owning slice | #16–18 above; #18 decided inside 13 before exit-path claims; #1/2 in slice 9, #3 in 11, #5 in 16, #17 with the first decision task. |
