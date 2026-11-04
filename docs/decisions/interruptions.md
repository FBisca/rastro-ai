# Interruptions

Status: agreed 2026-11-04. North star: `../concept.md` §6. Related: `renderers.md`, `learning.md`.

## Decisions
1. **Interruption is its own axis,** separate from act/propose/ask. It governs how much an item may break the owner's focus.

| Level | Meaning | Example surfaces |
|---|---|---|
| 0 Silent | Recorded only; visible when queried | Action-run log |
| 1 Ambient | Shows up in views; no signal | Vault view, dashboard, Herdr queue pane |
| 2 Nudge | A status or count changes | Herdr "needs you" status, menu bar badge |
| 3 Notify | A notification that doesn't block | Phone or desktop banner |
| 4 Interrupt | Time-sensitive; breaks focus | A grace window about to fire on an "others see it" action; someone waiting and it's due now |

2. **What sets the level:**
   - how urgent it is (time to the deadline or the end of a grace window);
   - how far it reaches;
   - whether a person is waiting;
   - the owner's interruption budget;
   - learned usefulness.
3. **Owner rules.** The owner can write explicit rules for other interrupts ("anything from X interrupts me"). These are rules, not learned.
4. **Learning.** Notification types the owner keeps ignoring drop a level. Raising a level is proposed to the owner, who confirms.
5. **Budget.** A cap on interrupts per period.
6. **Escalation.** An unanswered level-3 item escalates as its deadline gets close.
7. **Batching.** Levels 1–3 can be delivered in windows the owner chooses.
8. **Attention routing** (which views go to which surface, at what level) is owner config. Default: interrupt only for urgent questions and for grace windows on "others see it" actions.

## Open
- Focus state: whether it's read from the calendar and active sessions, or only from an explicit focus mode. See `../open-questions.md`.
