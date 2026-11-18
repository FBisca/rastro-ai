# Learning: trust and usefulness

Status: agreed 2026-11-04, except where marked open. North star: `../concept.md` §2 and §7. Related: `levels.md`.

## Decisions
1. **Learning lives in the graph.**
   - Outcomes are evidence, with the owner's behaviour as their source.
   - Levels and usefulness scores are claims derived from that evidence, with justifications.
   - Learning is deterministic, can be inspected and rebuilt, and is included in the export. No opaque machine learning.
2. **What gets recorded:**

| Item | Outcomes |
|---|---|
| Proposal | accepted unchanged · accepted with edits · rejected (optional reason) · ignored · became moot |
| Action run | kept · stopped during its grace window · undone · corrected later · "don't do this" |
| Question | answered · dismissed · "don't ask again" |
| Notification / view item | acted on within T · opened · dismissed · ignored |
| Missed | The owner reports something Rastro should have caught (a first-class intent) |

3. **Ignored requires that the owner saw it.** Renderers report what they showed. Unseen is not ignored.
4. **Absence pauses learning.** With no owner activity on any surface, nothing is learned from silence.
5. **Trust and usefulness are separate.**
   - **Trust:** should it do this alone? Kept per cell.
   - **Usefulness:** should it do this at all? Kept per plugin, workflow, view and notification type.
6. **Trust goes up slowly and down fast.**
   - **Up:** a cell becomes eligible for promotion after ≥10 clean outcomes over ≥7 days, with 0 undos in its last 20. These are configurable defaults.
   - **Down:** one undo or correction drops one level at once. Repeated ones drop the cell to Ask.
7. **An accept with edits isn't a clean outcome.** It teaches the action *how* (voice, fields) but doesn't count toward promotion.
8. **Trust spreads only with the owner's yes.** Rastro may propose widening a cell ("extend to all internal colleagues?").
9. **Unused trust decays.** A cell unused for long drops a level the next time it's used.
10. **Ceilings always win.** Learning never crosses reach and reversibility caps or the owner's rules.
11. **Usefulness is measured by:**
    - engagement (acted on ÷ seen);
    - time to action;
    - cost per accepted item;
    - duplicate/stale rate (the target is 0; above 0 is a quality failure);
    - missed reports.
12. **The kill rule is built in.** When usefulness stays low for W weeks, Rastro proposes narrowing, slowing down or removing the item. The owner decides.
13. **Ignoring something lowers its priority and interruption level first, not its trust.**
14. **Audit.**
    - **Trust view:** each cell's level, its evidence counts, the last change, and what the next promotion needs.
    - **Learning journal:** every level change, with its justification.
    - **Owner controls:** pin a level, reset a cell, set ceilings.
15. **Measuring the product itself.** The signals that "the owner can stop checking the sources" is happening:
    - missed reports trending to 0;
    - duplicate/stale items at 0;
    - review minutes a day;
    - the interruption budget respected;
    - the share of items decided by deterministic evidence rising.

16. **An outcome is derived, never a second table (2026-11-18).** The owner intent is the durable record; an outcome is read from the intents and the shown reports, and a claims rebuild reproduces both. A shown report has its own insert-only table because it records what the owner saw, not what Rastro knows. No surface writes an outcome.
17. **Seen is what the terminal printed (2026-11-18).** A terminal queue read records a shown report for each item it printed. The same read as `--json`, and the MCP tool, record none: automation cannot make an item look ignored.
18. **Ignored is shown, unanswered and gone (2026-11-18).** An item reads ignored when a terminal read recorded it, no intent answered it, and a later run of its source dropped its content; an item still pending is never ignored, however often it was shown. A run that kept the item's content — a replay, or a re-import of the same transcript — did not drop it and never dates it. Its outcome carries the time it waited, from its first shown report to the run that replaced its source's content. Engagement counts each item once: `shown` is what a terminal read printed, `acted on` is what an intent answered.
19. **Settled elsewhere is became moot, not ignored (2026-11-18).** An item a terminal read showed and no intent answered that the queue settled before any run dropped it became moot: the statements it expressed became known, so the queue left it out as already known, the owner accepted the same statements from another source, `identity link` merged its accounts, or `meeting forget` took its source away. It is a proposal outcome of its own, not ignored: the owner settled it, and counting it as ignored pushes Trust and Usefulness the wrong way. Which came first — a run that stopped carrying the item, or the moment its statements were known — fixes both the kind and the time, and no later read derives either again: an item a run really dropped stays ignored even when later knowledge holds its statements, and a settled item stays became moot even when a later run no longer carries it. A shown report carries the item's standard statements for exactly this, because the item is gone by the time an outcome is read. Nothing records when a settled item left, so its outcome is dated at the last shown report that printed it waiting and measures no wait; dating it at the read, or at a run that ran later, would move it after the fact. Moot is never counted as acted on.

## Experiments
- **Rubber-stamp guard.** Approvals given faster than the item could be read count as low-attention and carry little weight. To be tried and measured; drop it if it doesn't help.

## Open
- Whether ignoring an item means it's useless needs a deeper look. See `../open-questions.md`.
