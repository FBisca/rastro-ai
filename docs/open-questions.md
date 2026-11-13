# Open questions

Questions parked during the 2026-11-04 concept session. Each one moves into its decision file once it's decided.

| # | Question | Scope | Notes |
|---|---|---|---|
| 1 | Does an ignored item mean it's useless? | `decisions/learning.md` | Current assumption: ignoring something lowers usefulness and priority, not trust. Needs a deep dive |
| 2 | Rubber-stamp guard: discount approvals given too fast to have been read? | `decisions/learning.md` | Experiment: measure it, drop it if it doesn't help |
| 3 | Focus state: read from the calendar and active sessions, or only an explicit focus mode? | `decisions/interruptions.md` | |
| 4 | Plugin registry governance: open, curated, or both, with verified plugins marked? | `decisions/plugins.md` | Shapes the trust model |
| 5 | How shared claims get accepted (rule, trust index, other) | `decisions/sharing.md` | The owner's explicit acceptance is decided; the mechanism isn't |
| 9 | `rastro source remove`: config only, or does its evidence go too? | `decisions/knowledge.md` | Evidence is insert-only; forgetting is slice 14. How hits of a removed source state their coverage is decided (`decisions/knowledge.md` §21); the command itself is open. Re-adding the same name and scope inherits old windows; a different selector reuses `last_complete_check` as its resume point, leaving a history gap (accepted observation 2026-11-04; semantics still open) |
| 11 | Who may declare a vocabulary namespace, and so widen which types are accounts? | `decisions/plugins.md` | Plugin trust; left open by the owner on 2026-11-05. Plan D43 (owner Q22): `plans/2026-11-05-second-source-one-identity.plan.md` |
| 12 | Does an accepted meeting hypothesis become a claim, and under what acceptance rule? | `decisions/meetings.md` | Hypotheses are kept apart from the claims projection for now (`adr/0011-meeting-commitment-hypotheses.md`); the queue (slice 6) decides |

Decided: #6: a candidate whose party is mapped to a bot is excluded and counted; a party only named is kept (`decisions/knowledge.md` §71), 2026-11-13. #10: `about` says whether the identifier resolved, computed from visible data only, so a hidden entity still looks unknown (`decisions/knowledge.md` §57, `decisions/surfaces.md` §39), 2026-11-05.
