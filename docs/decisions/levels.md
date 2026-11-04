# Levels: act, propose, ask

Status: agreed 2026-11-04. North star: `../concept.md` §2. Terms: `../GLOSSARY.md`.

## Decisions
1. **Three levels.** Act, Propose, Ask.
   - **Act:** done visibly, with an undo where possible.
   - **Propose:** prepared, waiting for accept, edit or reject.
   - **Ask:** a question that can be answered in one step. The answer becomes a confirmation, so the same question is never asked twice.
2. **Act needs deterministic evidence.** An Act runs only when a rule is met by observed or confirmed claims. An agent's judgment alone never authorizes an Act; at most it proposes. A hypothesis can be proposed or asked about, never acted on.
3. **Every Action run stores its justification.** It records:
   - the rule;
   - the claims and evidence behind it;
   - the level it ran at;
   - the trust history of its cell.

   It can be queried from any surface ("why did you do X?", "what would make you do Y?"). The same facts plus the same rule always give the same decision.
4. **Actions declare their risk.** Each action states what it touches, its **reach** (only me → my things → others see it → public) and its **reversibility** (can be undone → can be compensated → can't be undone).
5. **The level is held per cell, meaning action × context** (person, project, source or channel). The most specific match wins.
6. **New contexts** inherit the general cell's level, capped at Propose for anything other people see.
7. **Promotion is always proposed to the owner, never self-applied.** Demotion is automatic and visible.
8. **Explicit rules.**
   - Irreversible or public actions, and Acts based on claims shared by another owner, reach Act only through a rule the owner writes: readable, versioned config, like Claude Code settings.
   - Learning never writes rules. It may propose edits to them.
   - How shared claims get accepted (a rule, a trust index, or something else) is an implementation choice. What is fixed is that the owner explicitly accepts them.
9. **Untrusted content never raises a level.** An action triggered by untrusted content is capped at Propose.
10. **Ask instead of Act** when evidence can't be verified: a source it couldn't reach, an unclear identity, or claims that contradict each other.
11. **Grace window.** An option on Act, with defaults by sensitivity:

| | Can be undone | Can be compensated | Can't be undone |
|---|---|---|---|
| Only me | none | short | explicit rule |
| My things | short | medium | explicit rule |
| Others see it | medium | long | explicit rule |
| Public | explicit rule | explicit rule | explicit rule |

   *short*, *medium* and *long* are durations the owner configures.
12. **Budgets.** The owner can cap Acts per period ("at most 5 sent messages a day").
