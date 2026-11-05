# Open questions

Questions parked during the 2026-11-04 concept session. Each one moves into its decision file once it's decided.

| # | Question | Scope | Notes |
|---|---|---|---|
| 1 | Does an ignored item mean it's useless? | `decisions/learning.md` | Current assumption: ignoring something lowers usefulness and priority, not trust. Needs a deep dive |
| 2 | Rubber-stamp guard: discount approvals given too fast to have been read? | `decisions/learning.md` | Experiment: measure it, drop it if it doesn't help |
| 3 | Focus state: read from the calendar and active sessions, or only an explicit focus mode? | `decisions/interruptions.md` | |
| 4 | Plugin registry governance: open, curated, or both, with verified plugins marked? | `decisions/plugins.md` | Shapes the trust model |
| 5 | How shared claims get accepted (rule, trust index, other) | `decisions/sharing.md` | The owner's explicit acceptance is decided; the mechanism isn't |
| 6 | Filtering bot-authored comments (deploy previews, CI bots) out of search or commitments | `decisions/knowledge.md` | Bots are kept as evidence with author claims (`knowledge.md` §27); filtering for usefulness stays open until slice 5 |
| 9 | `rastro source remove`: config only, or does its evidence go too? | `decisions/knowledge.md` | Evidence is insert-only; forgetting is slice 14. How hits of a removed source state their coverage is decided (`decisions/knowledge.md` §21); the command itself is open. Re-adding the same name and scope inherits old windows; a different selector reuses `last_complete_check` as its resume point, leaving a history gap (accepted observation 2026-11-04; semantics still open) |

Decided: #10 — `about` says whether the identifier resolved, computed from visible data only, so a hidden entity still looks unknown (`decisions/knowledge.md` §57, `decisions/surfaces.md` §39), 2026-11-05.
