# Export and sharing

Status: agreed 2026-11-04. North star: `../concept.md` §1 and §7.

## Decisions
1. **Export.** Evidence, confirmations, rules and outcomes export in an open, documented format. The knowledge rebuilds from them anywhere. No lock-in.
2. **Sharing grows out of export.** An owner shares a scoped set of claims with another owner.
3. **Shared claims arrive as evidence from a person, not as truth.** The receiver's Rastro weighs them by the receiver's trust in the sender, and can contradict them.
4. **Each owner stays sovereign:** no shared database, no shared truth. Households, friends and teams are owners exchanging scoped claims.
5. **A shared claim authorizes an Act only if the receiving owner explicitly accepts it.** It could be a rule ("trust Alex's calendar claims"), a trust index, or something else; the mechanism is an implementation choice. See `levels.md` §8.
6. **Revoking a share** stops further updates. Whatever the receiver already received, they keep.
7. **Monetization, if it ever happens,** fits as hosted sync and sharing on top of the open core.

## Added 2026-11-05 (knowledge graph deep dive)
8. **No standards commitment until slice 16.** Export is precious inputs (§1); claims are not exported, and no RDF, PROV or nanopublication profile, and no IRI scheme, is published before Share has a consumer. **Entry conditions for slice 16:**
    - hypotheses map to *unasserted* statements;
    - round-trip tests check proposition identity and status, not counts;
    - a statement-node JSON-LD 1.1 serializer sits at the edge;
    - which plugin owns a namespace prefix when two vocabularies collide (two `code` vocabularies) is decided then, and Share disambiguates by vocabulary id and version;
    - cross-machine tie-breaks at equal `captured_at` are settled with export in slice 13; import preserves export order.

## Added 2026-11-05 (roadmap slice 4)
Architecture: `../adr/0010-identity-and-owner-intents.md`.
9. **Owner intents export too (amends §1).** Evidence, confirmations, rules, outcomes and owner intents export; derived automatic links don't (`knowledge.md` §53).
