# 13 Identity, aliases, entity resolution and reversible merges (A4)

Researcher: `kg-identity`, step 1. Checked 2026-11-04. Nothing here is decided; the owner decides.

Working names in this file that are **not glossary terms**: *cluster* (the set of entities that active merges join), *representative* (the one id a cluster is shown under), *cannot-link* (a confirmed "these are different"), *account* (used loosely for `code:Account` and its future Slack, mail and calendar peers). If any of them survives, it goes through the glossary review (AGENTS.md).

## 1. Where Rastro stands today (repo facts)
| Fact | Where |
|---|---|
| One identity per entity, many aliases. A merge is a claim, so it can be undone. Exact identifiers merge automatically, fuzzy matches become an Ask, names alone never merge | `docs/decisions/knowledge.md:13-15` (§4) |
| One exact canonical identifier is one entity across sources and scopes; that is not a merge. `github:account/<numeric id>` with the login as alias; normalised `html_url` for PRs, issues, comments, repos | `knowledge.md:47` (§24), plan D7 |
| Repo rename or transfer and URL redirects give a new entity (accepted limit) | `knowledge.md:53` (§30) |
| Nothing is typed `Person`; slice 4 links accounts to Persons | `vocabulary.md` §12, plan D8 |
| Entity id = `e_` + first 16 bytes of SHA-256 over a versioned encoding of the canonical identifier. Claim id = hash(subject entity id, predicate, typed object) | `crates/rastro-core/src/claims.rs:74-99`, ADR 0009 |
| `entity(id, type, identifier UNIQUE)`, `alias(entity_id, alias, kind)`, `claim(subject → entity.id, …)` | `crates/rastro-storage-sqlite/migrations/202611061000_claims_projection.sql:7-19` |
| Every derived field, including resolution and aliases, is computed from supports visible to the caller's scope | ADR 0009 "Ids and revision", `surfaces.md` §26 |
| Intents target a claim id plus the revision the owner saw | `renderers.md:13` (§5) |
| An Ask's answer becomes a confirmation, so the same question is never asked twice. Unclear identity means Ask instead of Act | `levels.md:9`, `levels.md:27` (§10) |
| Knowledge = f(evidence, confirmations, rules); those plus outcomes are the only precious inputs | `knowledge.md:26` (§12) |

**The core tension:** the claim id hashes the *subject entity id*. Any merge design that rewrites a claim's subject to a merged id changes the claim id, and renderers target claim ids (§5). So the merge design and the id design are one decision.

## 2. How identity is modelled elsewhere
| System | Identity | Merge | Undo | Lesson for Rastro |
|---|---|---|---|---|
| **RDF/OWL** | IRIs. No unique-name assumption: two IRIs may denote one thing (OWL 2 Primer §4.7) | `owl:sameAs`: symmetric, transitive, and it lets a reasoner apply every statement about one name to the other | Retract the triple and re-reason. No provenance unless you reify | Full substitution plus transitivity is what makes errors spread (see "sameAs problem") |
| **"sameAs problem"** | 558,943,116 `owl:sameAs` statements over 179,739,567 names in 48,999,148 equivalence classes (sameAs.cc, Beek et al. 2018) | Estimated error rates 2.8% (Hogan 2012) to 20% (Halpin 2010). In Halpin's study three experts judging 250 links agreed on as few as 73 as correct | Detection after the fact: community structure of the identity graph flags bad links at 40–73% precision and 93% recall (Raad et al. ISWC 2018) | Weaker predicates (`skos:closeMatch`, `skos:exactMatch`, Halpin's `so:claimsRelated`) and *contextual identity* (identity only within a set of properties) exist but have little uptake (Raad et al. survey §6) |
| **schema.org** | `sameAs` = "URL of a reference Web page that unambiguously indicates the item's identity" | No reasoning semantics | n/a | A *pointer to an authority*, not an equivalence axiom. Closer to Rastro's alias |
| **Wikidata** | Q-ids, "designated as persistent identifiers". Never reused | Merge = move statements, then turn the loser into a **redirect**. The recipient is "usually the item that is used more often"; "when in doubt … the lowest Q####" | Restore the pre-merge revision on both items, in order. A bot rewrites statements that point at a redirect about 24 h later, so the delay is what makes a merge "easily reverted" | Old ids keep resolving forever. Rewriting references is a *separate, delayed* step. In WDQS the redirect is exposed as `owl:sameAs` |
| **LPG (Neo4j APOC)** | Internal node id plus a business-key property | `apoc.refactor.mergeNodes` folds the nodes onto the first one in the list and moves relationships; `properties: "combine"` turns conflicting values into a list | Not documented. In practice destructive | A destructive merge loses which edge came from which node |
| **Neo4j agent-memory** | Entity nodes | Deterministic shortcuts first (an exact normalised name or a declared alias scores 1.0); auto-merge at ≥0.90; between 0.85 and 0.90 a **pending `SAME_AS` edge** keeps both nodes for review; below 0.85, a new entity. "Name embeddings don't count as corroboration" | Not documented | The review band maps onto Rastro's Ask. A pending edge is a candidate, not a merge |
| **Graphiti / Zep** | UUID nodes per `group_id` | Candidates come from configured identity properties, then exact normalised name, then embeddings (cosine ≥0.6, top 15). Fuzzy merges need MinHash Jaccard ≥0.9 and pass an entropy gate (short or low-entropy names are skipped). Several exact-name matches never merge silently; they go to the LLM. Unresolved nodes go to the LLM. Records `IS_DUPLICATE_OF` edges between existing nodes. Zep added deterministic identity-property merges on 2026-07-09 | No undo in the resolution code | Even the LLM-first systems moved toward deterministic keys first, with the LLM last |
| **Segment Unify (CDP)** | Profile with external ids | Exact identifiers only. Limits per identifier type (defaults: `user_id` 1, others 5) plus a priority order; at a limit the lower-priority id is dropped. **Blocked values** (`null`, `-1`, `anonymous`, `^[0-]*$`, dev placeholders) never become identifiers | No unmerge is documented. External ids "remain for the lifetime of the user profile". Deletion is the whole profile (GDPR) | Limits and blocklists stop "superclusters" through shared identifiers |
| **Hightouch IDR** | `ht_id` per profile | Ordered rule sets (exact, fuzzy, Soundex) plus limit rules | "Reprocessing deletes the affected profiles' records and rebuilds them from your source data" | Identity as a *rebuildable projection* of source rows plus rules, like ADR 0009. Its docs disagree on whether `ht_id` stays stable across full re-runs |
| **Senzing** | Entity-centric | Claims "sequence neutrality": the same result whatever the record order, and it revisits earlier decisions (it splits a "father and son" once a new record reveals them) | Automatic re-resolution | Order-independence is the property a replayed projection needs. Vendor claim, unverified |
| **HubSpot CRM** | Record id | Merge into a primary record | "Merging contacts cannot be undone" (secondary source) | The baseline Rastro has to beat |
| **Apple Contacts** | One card per account | "Link" keeps each account's card; "Contacts shows just one card with the combined information" | Unlink per account. Removing an account unlinks and removes only that account's cards | A **non-destructive overlay** in a personal tool: the members stay intact, so unlinking is trivial |
| **Obsidian** | File path | `aliases` frontmatter list; a link is written as `[[Note|Alias]]` | n/a | Aliases are names for display and lookup, not identity evidence |

## 3. Merge strategies
| | A. Destructive merge | B. Union-find, re-keyed claims | C. Merge as claim, overlay (no re-key) | D. Merge as claim, re-key in projection |
|---|---|---|---|---|
| Stored | One surviving node | `parent` pointers; claims rewritten to the root | `same_as` claims with justifications; members and their claims untouched; a materialised member → cluster table | `same_as` claims; the projection re-derives claims with subject = representative |
| Undo | Not possible (HubSpot, APOC) | Union-find can't split. A split means recomputing the component from the remaining edges | Retract or deny the `same_as` claim, then recompute that one component. Member claims are untouched | Retract, then re-project the component; claim ids change back |
| Claim ids (ADR 0009) | Lost | Change on every merge | **Stable**: the subject stays the member entity | Change on merge and on unmerge; needs a claim-redirect table |
| Renderer targets (§5) | Break | Break | Hold. The merge itself is targeted through the `same_as` claim id and revision | Need redirects plus a forced revision conflict |
| "One claim, two supports" (§5) | Holds | Holds | **Strained**: A.`p`=x and B.`p`=x stay two claims. They have to be grouped at read time ("equal modulo cluster") | Holds |
| Rebuild, order | Order-dependent | Order-dependent if roots are chosen on arrival | Connected components over active `same_as` are order-independent | Same, if the representative is a pure function of the members |
| Cost at query time | None | One root lookup | One cluster lookup, then a union over the members' claims and a group-by for duplicates. Cheap at personal scale | None |
| Cost at projection time | n/a | Rewrite every claim on merge | Recompute one component | Re-derive every claim of the component on each merge or unmerge |
| Scope (§14) | Can't vary by scope | Hard | The cluster is computed from visible `same_as` supports, per scope, as ADR 0009 already does for aliases | Per-scope claim ids, or a full-scope id that leaks the merge |

**Dynamic connectivity** (Wikipedia): union-find handles insertions only. Deletions need heavier structures (Holm–de Lichtenberg–Thorup, amortised O(log² n)). At Rastro's scale, recomputing one component from its edges is enough, and that is how a projection works anyway.

### Choosing the representative (C and D)
| Rule | Deterministic on rebuild | Stable when a member joins | Stable on unmerge | Notes |
|---|---|---|---|---|
| min(member entity id) | yes | no: a joining member with a lower hash takes over | no, if the min member leaves | Hash order is arbitrary but pure. Hightouch uses "lowest id absorbs" |
| Earliest by replay order (`captured_at`, evidence id) | yes | yes, unless a backfill brings older evidence | no, if the earliest member leaves | Like Wikidata's "lowest Q" (the oldest item) |
| hash(sorted member set) | yes | no, it changes every time | no | Bad for references (cf. ABCDE id-stability metrics, van Staden 2024) |
| A Person minted from a confirmation (hash of the confirmation id) | yes: confirmations are precious inputs | yes | yes, while the confirmation stands | Only for owner-confirmed Persons |

Whatever the rule, if the representative is always one member's own entity id, then **every id ever handed out keeps resolving**, because a member id maps to its current cluster. That is a Wikidata redirect for free, with no redirect table, until the member is forgotten.

## 4. Undo and rebuild semantics (C or D)
- A `same_as` claim needs a symmetric id: hash(min(a,b), `same_as`, max(a,b)), so A~B and B~A are one claim with two supports.
- Justifications: *provenance* (an exact-identifier rule over structured evidence, e.g. two accounts sharing a source-verified email), *confirmation* (the owner's answer to an Ask), *inference* (later).
- **Undo has to be recorded as a negative confirmation** (a cannot-link, the analogue of `owl:differentFrom`). If it only retracts, the next rebuild re-derives the merge from the same exact-identifier rule. A negative confirmation is a precious input (§12), and it also keeps the Ask from being asked again (`levels.md` §1).
- A cluster is the connected components over the active `same_as` claims. A cannot-link between two members of one component (A~B~C, but A≠C) is a **conflict** (§8). The fix is an Ask about which link to drop, never a silent cut.
- After unmerge under C, nothing moves: the member claims were never rewritten. Under D, the component's claims are re-derived with their old subjects, and their old claim ids come back (content addressing), so an old renderer target becomes valid again.
- Actions: an Action run whose basis rested on a merge (e.g. "remind Ana" through account B) is flagged when the merge is undone, as for any retraction (§10).

## 5. Entity resolution practice
| Approach | How | Fit for Rastro |
|---|---|---|
| **Deterministic keys** | Equal normalised identifiers | Already §4 and §24. Needs a per-identifier, versioned normalisation (§7 below) |
| **Fellegi–Sunter** (Splink 5.0.0, 2026-09-28) | Per-field m/u probabilities. Bayes factor K = m/u; match weights add (assumes independence); a prior λ. Clusters by connected components at a threshold | Good for producing a *candidate* with a score. The output is a hypothesis, never a merge |
| **dedupe** 3.0.3 (2024-08, no release since) | Active learning; hierarchical clustering with centroid linkage, because pairwise probabilities "are not transitive" (A–B 60%, B–C 60% ⇒ A–C ≈ 36%) | The non-transitivity argument is the reason not to auto-close chains |
| **Zingg** 0.7.0 (2026-08-10) | Active learning on Spark | Too heavy for embedded use. Cluster ids changed between jobs (discussion #600) |
| **LLM-assisted** | Pairwise prompt, or the fallback (Graphiti) | OpenSanctions Pairs (2026): GPT-4o 99.0% F1 and a local 14B model 98.2%, against a rule baseline at 91.3%. Rules over-match; LLMs fail on transliteration and small identifier or date inconsistencies; "pairwise matching … approaching a practical ceiling", so the open problems are blocking, clustering and review. LLM self-explanations are "weakly faithful", "often unstable" (Teofili et al. 2026-05). Prompt choice depends on model and dataset (Peeters et al. EDBT 2025) |

**Failure modes that matter for one owner's graph**
- **Transitive chaining.** One false edge joins two dense groups (a "bridge" in Splink's graph metrics). Raad et al. use the same signal. A guard: auto-merge only on exact identifiers; flag any component with a bridge or above a size cap.
- **Shared identifiers.** `noreply@`, role mailboxes, `team@`, calendar rooms, a family's shared Apple ID, placeholder values (Segment's blocked list). A guard: a blocklist plus limits per identifier type, as Segment does. An identifier claimed by two Persons is a conflict and becomes an Ask.
- **Self-asserted identifiers.** A git commit author email or a mail display name can be anything the sender writes. Only identifiers the *source vouches for* (a Slack profile email under `users:read.email`, a GitHub numeric id, a verified address-book entry) should count as exact for auto-merge. Others are features for a candidate.
- **Recycled identifiers.** A released GitHub login "becomes available for anyone else to claim"; a work email gets reassigned. An alias therefore needs a known interval, and a lookup by alias has to handle "this alias meant two entities over time".
- **LLM rationale as justification.** Self-explanations are unfaithful, so a `why` should show the *evidence features* (both spans with the shared identifier), never the model's prose.

**"Names never merge" (§4) in practice:** a name can *raise* a candidate (blocking) but never *decide* one. Neo4j: "name embeddings don't count as corroboration". Graphiti: low-entropy or short names skip fuzzy matching, and several exact-name hits never merge silently.

**When a fuzzy match becomes an Ask (candidate criteria to discuss)**
1. A blocking hit on something stronger than a name: a shared non-verified email, the same signature block, co-occurrence in one thread, a matching phone number.
2. A score in a review band (Neo4j's default band is 0.85–0.90; Rastro would calibrate on labelled pairs).
3. **It matters now:** the pair blocks something (an Act's basis, a commitment's counterparty, `about` for a Person the owner asked about). Otherwise it stays a quiet hypothesis. This keeps Asks rare (interruptions).
4. One question, with both sides' evidence; both answers are stored (`same` → a confirmation on `same_as`; `different` → a cannot-link).

## 6. Accounts vs Persons
| Pattern | Model | For | Against |
|---|---|---|---|
| **P1. Accounts merge into each other** | `same_as` between `code:Account`, a Slack account, a mailbox… | Nothing new to add | An account isn't a human (`vocabulary.md` §12): bots, shared and organisation accounts. Merging two accounts asserts they are *the same account*, which is false |
| **P2. Person as a hub, accounts link to it** | Core `Person` entity; a claim (working name `held_by`) links each account to it; merges (`same_as`) happen *between Persons* | Matches §2 ("Person is in the core") and §12 ("slice 4 links accounts to Persons"). A shared account can link to two Persons without merging them. Apple Contacts is the same shape | Needs a rule for minting Person ids (§3 table) |
| **P3. Person = cluster** | No Person entity; a cluster of accounts is typed Person by a confirmation | Fewer entities | The Person's id moves with its representative; a shared account glues two humans together |
| **Address book as a source** | A contacts connector gives structured person records listing emails and phones | Person ids are source canonical ids (a contact's resource name); links to accounts are *exact*, with provenance | The owner's contacts are often stale or have duplicates themselves |

Canonical account identifiers, from primary docs:
- **GitHub user:** the numeric `id`. "Durable user ID instead of their login, which can change over time." Already §24.
- **Slack user:** `id` is "unique to the workspace", so the key is (`team_id`, `id`); for Enterprise orgs, prefer `enterprise_user.id`. `display_name` "may change at any time"; `name`: "Don't use this". `email` needs the `users:read.email` scope.
- **Email address:** normalisation depends on the provider. In personal Gmail, dots don't matter ("you own all dotted versions"); for Workspace and other organisations, "dots do change your address". So the normaliser is per provider and versioned, inside ADR 0009's canonical encoding.

## 7. Renames, redirects and transfers
| Event | What the source does | How other systems keep identity | Option for Rastro |
|---|---|---|---|
| GitHub login change | Old profile URL returns 404; the old login is released to others; repos redirect until someone reuses the name | Key on the numeric id | Done (§24). Add a known interval to the login alias, because of reuse |
| Repo rename | Web and git redirect; Pages and Actions don't; reusing the old name kills the redirect | — | Today a new entity (§30). Option: key repos on the numeric repo id (`GET /repositories/{id}`, unverified for transfers) with `html_url` as a time-bounded alias |
| Repo transfer | "All links to the previous repository location are automatically redirected"; a new repo at the old location deletes the redirects | — | Same as rename |
| Issue transfer | "The original URL redirects to the new issue's URL"; comments and assignees are kept | — | Either a stable source id (unverified) or a **redirect observation**: a connector that records "old URL → 301 → new URL" as structured evidence. An exact-identifier rule then turns it into a `same_as` claim with provenance. This is Wikidata's redirect-as-`sameAs` |
| URL in general | "Cool URIs don't change" (Berners-Lee 1998); in practice they do | Persistent ids plus redirects (Wikidata) | Don't use a URL as the canonical identifier when the source offers a durable id. Keep the URL as an alias |

## 8. Scopes, privacy and forgetting
- **A merge can be visible in one scope and not another**, and under ADR 0009 that is the default: a `same_as` claim is visible only through its visible supports. A merge justified only by work evidence leaves the two entities separate in a `--scope personal` read. That matches "a visible claim is never suppressed by a hidden replacement" (`surfaces.md` §26).
- **Open: the scope of a confirmation.** If the owner confirms "same person" while working in the work scope, does the merge show in personal? Options: the scope of the Ask's context, all scopes, or the owner chooses. Undecided anywhere.
- **Leak through the merge:** under C, a scoped read unions only claims with visible supports, so no hidden claim leaks. A cluster's *size* or representative id could still hint at hidden members. Fix: compute the cluster from visible `same_as` only, and pick the representative from visible members only.
- **Forgetting through a merge:** forgetting the evidence behind a `same_as` claim removes its provenance, so the merge dissolves unless another justification remains. "Forget this person" has to walk the cluster, or the `held_by` hub, and forget each member's evidence. CDP practice: re-ingestion recreates deleted profiles (Lytics), and Hightouch rebuilds from source tables, so a **forget marker** (§2) on the identifier itself has to block re-resolution, and the connector shouldn't re-capture it.
- **Erasing one identity in a merged profile** (Acquia allows it) leaves the other identifiers linked. A partial forget is a real case.

## 9. Mapping to Rastro's constraints
| Constraint | A destructive | B union-find re-key | C claim plus overlay | D claim plus re-key |
|---|---|---|---|---|
| Rebuildable projection (§12, ADR 0009) | ✗ | ~ (order-dependent) | ✓ | ✓ |
| Content-addressed ids (D5) | ✗ | ✗ (ids churn) | ✓ (unchanged) | ~ (ids change with membership, but deterministically) |
| Scope filtering of metadata (§14, S28) | ✗ | ✗ | ✓ (per-scope components) | ~ (scoped claim ids or a leak) |
| Renderers target claim id plus revision (renderers §5) | ✗ | ✗ | ✓ | ~ (needs redirects) |
| "One claim, two supports" (§5) | ✓ | ✓ | ~ (grouped at read time) | ✓ |
| Undo (§4, glossary "Merge") | ✗ | ~ | ✓ | ✓ |
| Query cost | low | low | cluster lookup plus union, small | low |

**Leaning (kg-identity, not a decision):** C. Merges are `same_as` claims with a symmetric id and justifications. The cluster is materialised in the projection and recomputed per scope from visible supports. The representative is a member's own entity id, chosen by a pure rule. Undo is a cannot-link confirmation. Persons follow P2, a hub that accounts link to. Auto-merge happens only on source-vouched exact identifiers, with a blocklist and limits per identifier type. Every fuzzy candidate stays a hypothesis until an Ask that matters. The cost is the read-time grouping that §5 ("one claim, two supports") needs across members. That grouping is a design gap for `20-*`/`21-*` to close.

## 10. Questions for the owner
1. Overlay (C) or re-key (D)? This is the trade between claim-id stability and §5's "no duplicate claims".
2. Person model: hub (P2), cluster (P3), or address book first?
3. Which identifiers count as exact for auto-merge? Only source-vouched ones, or also self-asserted ones like commit emails?
4. Is undo a negative confirmation (cannot-link) that survives rebuilds?
5. Which scope does an owner's merge confirmation carry?
6. Keep §30 (rename gives a new entity), or key repos and issues on source ids or redirect observations?
7. What does "forget a Person" do across a cluster?

## Sources (all checked 2026-11-04)
- OWL 2 Primer (2nd ed., 2012-12-11) §4.7: https://www.w3.org/TR/owl2-primer/
- Raad, Pernelle, Saïs, Beek, van Harmelen, "The sameAs Problem: A Survey on Identity Management in the Web of Data", arXiv 1907.10528 (2019-07-24): https://arxiv.org/abs/1907.10528 and https://ar5iv.labs.arxiv.org/html/1907.10528
- Raad et al., "Detecting Erroneous Identity Links on the Web Using Network Metrics", ISWC 2018: https://hal.inrae.fr/hal-02733996
- schema.org `sameAs`: https://schema.org/sameAs
- Wikidata Help:Merge: https://www.wikidata.org/wiki/Help:Merge ; Help:Redirects: https://www.wikidata.org/wiki/Help:Redirects
- Neo4j APOC `mergeNodes`: https://neo4j.com/docs/apoc/current/overview/apoc.refactor/apoc.refactor.mergeNodes/
- Neo4j agent-memory, resolution and deduplication: https://neo4j.com/labs/agent-memory/explanation/resolution-deduplication/
- Graphiti v0.31.2 (2026-10-20) source: https://github.com/getzep/graphiti/blob/main/graphiti_core/utils/maintenance/node_operations.py , `dedup_helpers.py`, `edge_operations.py` (`IS_DUPLICATE_OF`)
- Zep changelog 2026-07-09: https://help.getzep.com/changelog/2026/7/9
- Segment Unify FAQs: https://www.twilio.com/docs/segment/unify/faqs ; Identity Resolution Settings: https://www.twilio.com/docs/segment/unify/identity-resolution/identity-resolution-settings
- Hightouch IDR (docs are login-walled; search snippets only): https://hightouch.com/docs/identity-resolution/how-it-works , https://hightouch.com/docs/identity-resolution/profile-reprocessing
- Lytics GDPR: https://www.contentstack.com/docs/lytics/gdpr-compliance-with-cdp ; Acquia erasure: https://docs.acquia.com/customer-data-platform/using-360-profiles-data-erasure-requests
- Senzing: https://senzing.com/er-capabilities (marketing)
- Apple Contacts, link cards: https://support.apple.com/guide/contacts/adrb33f38d93/mac
- Obsidian aliases: https://obsidian.md/help/aliases
- Splink Fellegi–Sunter: https://moj-analytical-services.github.io/splink/topic_guides/theory/fellegi_sunter.html ; clustering API: https://moj-analytical-services.github.io/splink/api_docs/clustering.html ; graph metrics: https://moj-analytical-services.github.io/splink/topic_guides/evaluation/clusters/graph_metrics.html ; PyPI splink 5.0.0 (2026-09-28)
- dedupe, grouping duplicates: https://docs.dedupe.io/en/latest/how-it-works/Grouping-duplicates.html ; PyPI dedupe 3.0.3 (2024-08-15)
- Zingg docs: https://docs.zingg.ai/latest/ ; v0.7.0 (2026-08-10); discussion #600: https://github.com/zinggAI/zingg/discussions/600
- van Staden, "Evaluation of Cluster Id Assignment Schemes with ABCDE", arXiv 2409.18254 (2024-09-26): https://arxiv.org/abs/2409.18254
- Smith et al., "OpenSanctions Pairs", arXiv 2603.11051 (v1 2026-02-24): https://arxiv.org/abs/2603.11051
- Teofili et al., "Can we trust LLM Self-Explanations for Entity Resolution?", arXiv 2606.01210 (2026-05-31): https://arxiv.org/abs/2606.01210
- Peeters, Steiner, Bizer, "Entity Matching using LLMs", EDBT 2025: https://arxiv.org/abs/2310.11244
- Dynamic connectivity: https://en.wikipedia.org/wiki/Dynamic_connectivity
- GitHub, renaming a repository: https://docs.github.com/en/repositories/creating-and-managing-repositories/renaming-a-repository ; transferring a repository: https://docs.github.com/en/repositories/creating-and-managing-repositories/transferring-a-repository ; transferring an issue: https://docs.github.com/en/issues/tracking-your-work-with-issues/administering-issues/transferring-an-issue-to-another-repository ; changing your username: https://docs.github.com/en/account-and-profile/setting-up-and-managing-your-personal-account-on-github/managing-personal-account-settings/changing-your-github-username ; users REST API: https://docs.github.com/en/rest/users/users
- Slack user object: https://docs.slack.dev/reference/objects/user-object
- Gmail dots: https://support.google.com/mail/answer/7436150
- Berners-Lee, "Cool URIs don't change" (1998): https://www.w3.org/Provider/Style/URI

## Unverified
- Whether a repository's numeric `id` and an issue's `id`/`node_id` survive transfer (rename is very likely; transfer not confirmed in docs).
- HubSpot "merging contacts cannot be undone": seen in search snippets of HubSpot API docs and the community, not on a primary page fetched here.
- Hightouch `ht_id` stability across full re-runs: its own pages disagree; the docs are login-walled.
- Senzing sequence neutrality and real-time unmerge: vendor claims only.
- The size and contents of sameAs.cc's largest equivalence class (often quoted as very large and absurd; not confirmed).
- Mem0's entity dedup: no primary source on canonicalisation. PyPI shows `mem0ai` 2.2.1 (2026-09-25) as latest, which conflicts with the "Mem0 v3" naming in `knowledge-graphs-2026-11.md`; not resolved.
- Gmail plus-addressing as an alias rule (not checked); email normalisation for Outlook and iCloud.
- Graphiti: whether `IS_DUPLICATE_OF` edges are written for every LLM-resolved duplicate, or only between nodes that already existed.
- Whether git commit emails can serve as an exact identifier when GitHub marks a commit "Verified" (signature-based); not researched.
