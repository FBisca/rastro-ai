# 00: Current state (ground truth)

Agent: `kg-archaeologist`, 2026-11-04. Facts only; no recommendations. Repo HEAD `4f138e3` (main). Paths are relative to the repo root unless noted. `SS` = `crates/rastro-storage-sqlite`, `CORE` = `crates/rastro-core`, `SDK` = `crates/rastro-plugin-sdk/src/lib.rs`, `MIG` = `SS/migrations`.

---

## 1. Storage on main

Engine: SQLite through `rusqlite`, bundled. Migrations are compiled in (`SS/src/lib.rs:20-49`) and tracked by name in `schema_migrations(name PK, applied_at)` (`SS/src/lib.rs:93-96`). A writing open sets `foreign_keys = ON` and `journal_mode = WAL` (`SS/src/lib.rs:73,80`). The read-only open uses `SQLITE_OPEN_READ_ONLY` with no migrations (`SS/src/lib.rs:143-157`). Each read runs in one deferred transaction plus a migration check (`SS/src/lib.rs:160-170`).

### 1a. Sources, runs, coverage

| Table | Columns | Constraints / indexes | Ref |
|---|---|---|---|
| `sources` | `name` PK, `connector`, `selector`, `scope`, `freshness_rule`, `window_start`, `keep_original` (0/1), **`extractor`** (nullable, added in slice 3) | `CHECK keep_original IN (0,1)` | `MIG/202611041200_initial.sql:1-5`; `MIG/202611061000_claims_projection.sql:5` |
| `runs` | `id` PK, `source`→sources, `scope`, `plugin_id`, `plugin_version`, `started_at`, `finished_at`, `status`, `reason`, `submitted`, `accepted`, `rejected`, `unchanged`, `window_start`, `window_end`, **`role`** | `role IN ('connector','extractor')`, default `connector`. Index `runs_by_pair(source, scope)` | `initial.sql:6-12`; `202611050900_run_window.sql:3-4`; `claims_projection.sql:2-3`; `202611051300_coverage_pair_indexes.sql:1` |
| `coverage` | `id` PK, `run_id`→runs, `source`→sources, `scope`, `window_start`, `window_end`, `selection`, `state`, `reason`, `retry_at`, `checked_at` | `state IN ('found','empty','couldn''t look')`. Index `coverage_by_pair(source, scope, id)` | `initial.sql:13-19`; `coverage_pair_indexes.sql:2` |

### 1b. Evidence

| Table | Columns | Constraints / indexes | Ref |
|---|---|---|---|
| `evidence` | `id` INTEGER PK, `source`→sources, `scope`, `location`, `field`, `start`, `end`, `quote`, `captured_at`, `source_updated_at`, `trust_class`, `original` | `start>=0`, `end>=start`, `length(CAST(quote AS BLOB)) = end-start`, `trust_class IN ('structured','unstructured','untrusted content')` | `initial.sql:20-27` |
| (triggers) | `evidence_no_update`, `evidence_no_delete`, `evidence_no_replace` | each raises `'evidence is insert-only'` | `initial.sql:28-31`; `202611041500_evidence_insert_guard.sql:1-3` |
| `run_evidence` | `run_id`→runs, `evidence_id`→evidence | PK `(run_id, evidence_id)`, WITHOUT ROWID | `202611051100_run_evidence.sql:3-7` |
| (index) | `evidence_latest_by_location(source, scope, location, field, id)` | | `run_evidence.sql:8` |
| (index) | `evidence_by_pair_capture(source, scope, captured_at, id)`: replay order | | `claims_projection.sql:36` |

- Evidence has no kind column. The connector's `kind` is checked against the grant and then dropped (plan, line 30; ADR 0008 §2).
- Re-observing identical content only links it through `run_evidence` (`SS/src/lib.rs:271-301`).
- Within a (source, scope) pair, `captured_at` never goes backwards: it is clamped to the pair's maximum (`SS/src/lib.rs:303-312`).

### 1c. FTS

| Object | Definition | Ref |
|---|---|---|
| `evidence_fts` | `fts5(quote, content='evidence', content_rowid='id', tokenize='porter unicode61 remove_diacritics 2')` | `202611041700_evidence_full_text_index.sql:2-6` |

- Written by hand on append (`SS/src/lib.rs:332-335`); there are no sync triggers.
- `index rebuild` drops it and re-runs the migration (`SS/src/lib.rs:402-413`).
- Only evidence quotes are indexed. Entities, aliases and claims have no FTS.

### 1d. Projection (claims), migration `202611061000_claims_projection.sql`

| Table | Columns | Constraints / indexes | Line |
|---|---|---|---|
| `entity` | `id` TEXT PK, `type`, `identifier` | `identifier UNIQUE` | 7-9 |
| `alias` | `entity_id`→entity, `alias`, `kind` | PK `(entity_id, alias, kind)`, WITHOUT ROWID. **No uniqueness on `alias` alone** | 10-13 |
| `claim` | `id` TEXT PK, `subject`→entity, `predicate`, `object_entity`→entity, `object_value`, `object_datatype`, `status`, `revision` | `status IN ('observed','superseded')`. CHECK: the object is an entity XOR (value AND datatype) | 14-19 |
| `claim_support` | `claim_id`→claim, `evidence_id`→evidence, `field`, `start`, `end`, `extraction_rule`, `extractor`, `extractor_version`, `run_id`→runs, `current` (0/1, default 1) | PK `(claim_id, evidence_id, field, start, end, extraction_rule)`, WITHOUT ROWID. `end > start` | 20-27 |
| (indexes) | `claim_by_subject(subject)`, `claim_by_object(object_entity)` | No index on predicate or literal value | 28-29 |
| `extraction_checkpoint` | `extractor`, `extractor_version`, `source`, `scope`, `last_captured_at`, `last_evidence_id` | PK `(extractor, extractor_version, source, scope)` | 30-34 |

What main lacks, though ADR 0009 lists it: `known_interval`, `claim_link`, a generation, `valid_time`, and any scope column on entity, claim or alias (scope comes only through `claim_support → evidence.scope`).

---

## 2. Claim path on main

### 2a. Protocol (SDK, protocol `1.0`, additive)
| Message | Direction | Shape | Ref |
|---|---|---|---|
| `initialize` (extractor) | core → ext | `protocol`, `manifest_id`, `role: extractor`, `run: {run_id, extractor_version, pairs: [SourceScope]}`. The host always sends **one** pair | `SDK:144-169`; `CORE/src/host/extract.rs:171-183` |
| `evidence.batch` (notification) | core → ext | `items: [BatchItem{evidence_id, source, scope, location, span{field,start,end,quote}, captured_at, source_updated_at?, trust_class}]`, `last: bool`. 100 per batch, all queued up front, ordered by (`captured_at`, id) | `SDK:172-197`; `extract.rs:22,187-202` |
| `claim.submit` (notification) | ext → core | `subject: EntityRef{identifier, type}`, `predicate`, `object: {kind: entity, identifier, type} \| {kind: literal, value, datatype}`, `extraction_rule`, `supports: [SupportRef{evidence_id, field, start, end}]` | `SDK:199-236` |
| `run.finish` (request) | ext → core | `status`, `submitted`, `reason?`. The core answers with its own `Counts{submitted, accepted, rejected, unchanged}` | `SDK:307-321`; `extract.rs:240-276` |

- Manifest additions: `extraction_rules[{id, deterministic}]`, `vocabulary` (a path), `dependencies`, `default_extractor` (`SDK:336-363`).
- Vocabulary file on main: `{namespace, version, types: [String], predicates: [{id, domain[], range[]}]}` (`SDK:370-385`).

### 2b. Host and validation
- **Binding.** After each connector run, `sync` writes `sources.extractor = manifest.default_extractor` and then runs that one extractor (`CORE/src/runs/mod.rs:107-118`). That makes one extractor per source.
- **Vocabulary load.** Only the extractor's own file is read. It must sit inside the plugin directory, its namespace must be in `dependencies`, and every type and predicate id must be prefixed `namespace:` (`extract.rs:99-127`). Cross-namespace use is impossible today.
- **Pending evidence.** Everything in the pair past the checkpoint for (extractor, **version**, source, scope), all trust classes (`SS/src/lib.rs:443-487`).
- **Cap.** max(1000, 20 × delivered) claims per run, or the protocol fails (`extract.rs:25-26,167-170,213-217`).
- **`claims::validate`** (`CORE/src/claims.rs:132-234`) rejects a claim unless:
  - every term is ≤ 4096 bytes (`:143-151`);
  - the rule is declared and deterministic (`:152-159`);
  - the predicate is in the vocabulary (`:160-164`);
  - the subject is non-empty, its type is a vocabulary type and it is in the predicate's domain (`:165-172`);
  - an entity object has a vocabulary type in the range; a literal has a datatype that is **not** a vocabulary type and is in the range (`:173-194`);
  - there is at least one support (`:195-197`);
  - each support's evidence was delivered in this run, is `structured`, has the same field, has its span inside the evidence span and sits on char boundaries (`:198-225`).
- **Ids** (`claims.rs:74-101`). SHA-256 over a tagged, length-prefixed encoding, truncated to 128 bits:
  - `entity_id = e_ + H("rastro/entity/v1", identifier)`; the type is not in the id;
  - `claim_id = c_ + H("rastro/claim/v1", subject_id, predicate, entity_id | (datatype, value))`. Scope, source, extractor and vocabulary version are not in the id;
  - `revision = r_ + H("rastro/revision/v1", status, sorted current supports)` (`:104-128`).
- **In-run dedup.** The same claim id merges supports and counts as `unchanged` (`extract.rs:204-233`).
- **Commit.** Only on `run.finish{status: ok}`: `record_claims` writes the claims and the checkpoint (last delivered `(captured_at, id)`) in **one transaction** (`extract.rs:244-263`). A failed run stores nothing.

### 2c. Projection write (`SS/src/lib.rs:488-614`)
1. A type clash (stored `entity.type` ≠ submitted type) rejects the claim (`:510-525`). On main, an entity's type is fixed by its first claim.
2. `INSERT OR IGNORE` into `entity`, plus an `alias(entity, identifier, 'canonical')` row (`:526-536`). Main writes no other alias kind.
3. `INSERT OR IGNORE` into `claim` with status `observed` (`:537-549`).
4. `INSERT OR IGNORE` into `claim_support` for each support (`:550-567`). Nothing new means `unchanged`.
5. The revision is recomputed from `current = 1` supports (`:572-594`). Main never sets `current = 0`, so nothing is ever `superseded`.
6. Checkpoint upsert (`:596-611`).

### 2d. `why` on main
- Catalog: `why`, slot 6, **CLI only**, `read_only: true`, with no `reader:`, so it isn't on the read-only path (`crates/rastro-operations/src/catalog.rs:251-253`). The CLI uses the writing open (`crates/rastro-cli/src/main.rs:217-218`).
- `Rastro::why` errors on an unknown id (`CORE/src/lib.rs:453-457`). There is **no scope filtering**.
- Returns `Why{claim: ClaimView{id, subject: Entity{id, identifier, type}, predicate, object, status, revision}, supports: [SupportView{evidence_id, source, scope, location, field, start, end, quote (the support's sub-slice), trust_class, captured_at, source_updated_at, extraction_rule, extractor, extractor_version}]}`. Supports are ordered by `captured_at, id`, with no limit (`CORE/src/claims.rs:236-271`; `SS/src/lib.rs:625-733`).
- `about` does not exist on main.

---

## 3. Unmerged branches (both from `1fb2108`)

### T4 `feat/claim-lifecycle` @4ac017f: 17 files, +891/−40
Key files: `MIG/202611061100_claim_lifecycle.sql` (new), `SS/src/lifecycle.rs` (new, 284 lines), `CORE/src/claims.rs`, `CORE/src/runs/mod.rs`, `SDK`, CLI and catalog (`claims.rebuild`, slot 7, CLI only), `schema/operations-v2.schema.json`, `crates/rastro-cli/tests/claims.rs` (new).

| Adds | Detail | Ref (T4 worktree) |
|---|---|---|
| `claim.scalar` | 0/1, taken from the predicate's cardinality | `202611061100_claim_lifecycle.sql:1`; `claims.rs:243` |
| `claim.valid_time` | `NOT NULL DEFAULT 'unknown' CHECK (= 'unknown')` | migration `:2` |
| `known_interval` | `(claim_id, evidence_id)` PK, `start`, `end` nullable | migration `:3-7` |
| `claim_link` | `(from_claim, to_claim, kind IN ('conflicts_with','supersedes'))` PK | migration `:8-12` |
| `projection_generation` | a single-row counter, `id` starting at 1, incremented on publish. **Not** generation-keyed tables | migration `:13-14`; `lifecycle.rs:277` |
| `Predicate.cardinality` | `one` / `many`, default **`many`** | `SDK:383,391` |
| Lifecycle recompute | `lifecycle::refresh` runs on **every** `record_claims` and replays **all** `claim_support ⋈ evidence` in (`captured_at`, id) order (`lifecycle.rs:45-201`):<br>- deletes all intervals and links, then rebuilds them;<br>- a slot is (source, scope, subject, predicate, location, field, rule) plus the claim id when the predicate isn't scalar;<br>- "newer comparable" means a strictly greater `source_updated_at` (`:96-102`);<br>- a conflict is two scalar claims current on the same (subject, predicate, location, field) (`:132-141`);<br>- `supersedes` covers scalar claims only (`:121-131`) | `SS/src/lib.rs:581` |
| Revision with links | `revision_with_links` appends sorted (kind, target) pairs under the same `rastro/revision/v1` tag (the output is unchanged when there are no links) | `claims.rs:110` |
| `ClaimView` additions | `known_intervals`, `valid_time: unknown`, `conflicts_with`, `supersedes`, `superseded_by`; `SupportView.current` | `claims.rs:254-277` |
| `claims rebuild` | Takes `sync.lock`. Creates TEMP copies of entity, alias, claim, claim_support, known_interval, claim_link and extraction_checkpoint with their FK clauses stripped. Replays every bound source. On success, DELETEs `main.*` and INSERT…SELECTs from temp in one transaction. Runs stay outside the generation | `runs/mod.rs:126-157`; `lifecycle.rs:233-284` |
| `extraction_sources` | Builds every source as `Source::github(...)`: the connector is hard-coded | `SS/src/lib.rs:601-612` |
| `why` | Wrapped in `unchecked_transaction` | `SS/src/lib.rs:621` |

Status (plan, line 136): local gate 182 passed, converge PASS, no code review.

### T5 `feat/github-extractor` @7a532bb: 14 files, +1699/−41
Key files: the new crate `crates/rastro-extractor-github/` (manifest, `vocabulary.toml` of 109 lines, `src/main.rs` of 381 lines, and tests), `SDK`, `CORE/src/claims.rs`, `CORE/src/host/extract.rs`, `SS/src/lib.rs`, `crates/rastro-test-extractor/vocabulary.toml`. **No migration.**

| Adds | Detail | Ref (T5 worktree) |
|---|---|---|
| Meta-predicate `type` | A literal of datatype `type` whose value equals the subject's type. It is the only claim allowed to change `entity.type` (`UPDATE entity SET type`), so **entity type becomes mutable** | `CORE/src/claims.rs:16-17,177-182`; `SS/src/lib.rs:559,578` |
| Meta-predicate `alias` | A literal whose datatype is a vocabulary alias kind, the subject in the kind's domain, and the value exactly equal to text a support quotes. It also inserts into the `alias` table, never removed ("a renamed account keeps its old login") | `claims.rs:21,183-196,257-270`; `SS/src/lib.rs:585` |
| Type claims first | At commit, `type` claims are ordered before the rest | `extract.rs:270-275` |
| **Location context** | New `Storage::location_context`: for every location with pending evidence, the latest observation of each field up to the checkpoint that has no newer observation. It is **delivered** ahead of the pending items, and counts as "delivered" for support validation and the claim cap. This widens ADR 0008's "evidence delivered to this run"; no ADR records the change | `CORE/src/lib.rs` (trait); `SS/src/lib.rs:490-522`; `extract.rs:61-63,136-153` |
| **Vocabulary format** | `types: [TypeDefinition{id, definition, not_to_confuse_with, examples[], counter_examples[], evidence_requirement}]`<br>`predicates[]` add the same definition fields plus `freshness: source`, `authority: newer_comparable_observation`, `sensitivity`<br>`aliases: [AliasKind{kind, domain[], definition…}]`<br>`Vocabulary::check` refuses blank fields, an empty domain or range, or a domain type not defined in the file | `SDK:373-480` |
| `code` vocabulary | Types: `code:Account`, `Bot`, `Repository`, `PullRequest`, `Issue`, `Comment`<br>Predicates: `code:state` (string), `code:author` (→ Account/Bot), `code:in_repository` (→ Repository)<br>Alias kind: `login`<br>**No `cardinality` field** | `crates/rastro-extractor-github/vocabulary.toml:12-109` |
| Extractor rules | `state`, `kind-from-url`, `repository-from-url`, `author`, `account-type`, `login`. Reads only `/state`, `/user/id`, `/user/login`, `/user/type` | `rastro-extractor-github/rastro-plugin.toml:15-37`; `src/main.rs:49` |

Status (plan, line 137): local gate passed, converge **FAIL** (7 unit tests untagged), no code review.

### Overlap and conflict between T4 and T5
- `git merge-tree 4ac017f 7a532bb` merges **textually clean** (tree `6358157`), but the result **does not compile**. T4's `scalar: predicate.cardinality == One` (merged `CORE/src/claims.rs:288`) uses `predicate`, which T5 moved inside the generic match arm (merged `:211-212`). The type and alias meta-claims have no `Predicate`, so neither branch defines their cardinality.
- T5's `code` vocabulary has no `cardinality`, and T4 defaults to `many`. After a merge, `code:state` would never supersede or conflict unless the vocabulary is edited. T4's `rastro-test-extractor/vocabulary.toml` does set `cardinality = "one"`.
- Both change `Predicate`, `validate`, `ObservedClaim`, the `Storage` trait (T4: `extraction_sources`, `begin/finish_claims_rebuild`; T5: `location_context`) and `record_claims`.
- T5's in-place `entity.type` update and its `alias` table inserts sit outside T4's lifecycle recompute. Aliases never close, and type changes leave no interval.
- T4's rebuild TEMP-copies `alias` and `entity`, so T5's side effects are replayed only to the extent that replay re-derives them.
- T4's hard-coded `Source::github` in rebuild is independent of T5's extractor.

---

## 4. Real data (owner's live database, opened read-only)

- **Location.** No `RASTRO_HOME` or XDG overrides are set, so the database is `~/.local/share/rastro/rastro.sqlite3` (3.7 MB, last modified 2026-11-04 18:37) and the config is `~/.config/rastro/config.toml` (resolution: `CORE/src/lib.rs:123-148`). I opened it as `file:…?mode=ro&immutable=1`. No `-wal` or `-shm` files existed and none were created.
- **Schema.** 6 migrations, up to `202611051300_coverage_pair_indexes`. **`202611061000_claims_projection` isn't applied**, so there are no claim, entity, alias, support or checkpoint tables. Journal mode is still `delete`.
- **Installed plugins.** Only `~/.config/rastro/plugins/github` (connector 0.1.0, no `default_extractor`). No extractor is installed. The repo's connector manifest on main names `rastro-extractor-github` (`crates/rastro-connector-github/rastro-plugin.toml:11`), which exists only on T5.

| Table | Rows |
|---|---|
| sources | 3 |
| runs | 5 (3 `failed: interrupted`, 2 `ok`) |
| coverage | 5 (3 `couldn't look: interrupted`, 2 `found`, selection "involving the owner") |
| evidence | 1520 |
| run_evidence | 1648 |
| evidence_fts | 1520 |
| claims / entities / aliases | **0** (the tables don't exist) |

| Source | Scope | Repo | Evidence | Distinct locations |
|---|---|---|---|---|
| `kalios` | work | KaliosFit/app-flutter | 473 (148 structured, 325 untrusted) | 148 / 178 |
| `kalios-api` | personal | FBisca/app-nextjs | 1047 (258 structured, 789 untrusted) | 258 / 536 |
| `kalios-mobile` | personal | KaliosFit/app-flutter (**the same repo as `kalios`, other scope**) | 0 (never synced) | — |

| Field | Trust class | kalios | kalios-api |
|---|---|---|---|
| `/body` | untrusted content | 177 | 531 |
| `/title` | untrusted content | 148 | 258 |
| `/state` | structured | 148 (all `closed`) | 258 (257 `closed`, 1 `open`) |

- **Locations.** 714 distinct, all pull requests: 1212 item-level rows and 308 rows at comment URLs (`…/pull/N#issuecomment-…`). No issues.
- **History.** No (source, location, field) has more than one observation, so there is no supersession history in the data yet.
- **Other fields.** `source_updated_at` is set on all 1520 rows; `original` is set on none (default pointer plus span). There is no `/user/*` evidence, so author claims need a T2 connector plus `sync --reread` (plan D11, D12).

---

## 5. Decided vs open, per area

| Area | Decided (quote / ref) | De facto in code, not recorded as a decision | Open |
|---|---|---|---|
| **A1 Data model** | "Sources → Evidence (never changes) → Knowledge (claims about entities, derived and rebuildable)" (`decisions/knowledge.md:6`).<br>Claim = "status … and at least one justification"; "two sources say the same thing … one claim with two supports" (`knowledge.md:15-16`).<br>The ADR 0009 table list (`adr/0009:10-19`).<br>SQLite "covers … 1–2 hop link walks (recursive CTEs)" (`adr/0002:8`) | **Relational claim tables in SQLite.**<br>Claims are strictly **binary** (S, P, O), with O = entity XOR typed literal (`claims_projection.sql:14-19`).<br>No qualifiers on a claim besides supports.<br>Claim-about-claim exists only as `claim_link` (T4).<br>Datatypes are free strings with no registry (`claims.rs:188-189`) | RDF, LPG, n-ary or hypergraph is not chosen.<br>How merges (claims between entities), inference (claims resting on claims) and confirmations attach to claims |
| **A2 Reification and provenance** | Justification kinds: provenance, inference, confirmation (`GLOSSARY.md:17`).<br>"No span, no claim" (`knowledge.md:17`).<br>Span = field + UTF-8 byte range + quote (`adr/0005:8`).<br>Core validation list (`adr/0008:26-34`).<br>`why` shape (`decisions/surfaces.md:57`; plan D16) | Only provenance exists, as `claim_support` rows (claim, evidence, span, rule, extractor, version, run).<br>There is no generic justification table.<br>The justification row *is* the support | Storage for inference and confirmation justifications.<br>Chains of claim → claim.<br>Whether supports and justifications are one structure |
| **A3 Temporal** | Valid and known time; "as of" is supported (`knowledge.md:18`).<br>Known time comes from evidence `captured_at`; valid time is `unknown` (`knowledge.md:48`; `adr/0009:16`; plan D6).<br>Supersession rules (`knowledge.md:49`).<br>"As of" waits for slice 7 (`roadmap.md:24`) | Main has no time model on claims.<br>T4: `known_interval` per (claim, establishing evidence), and `valid_time` is constant `'unknown'`.<br>Comparability = `source_updated_at` strictly greater (`lifecycle.rs:96-102`).<br>Replay order (`captured_at`, id) is monotonic per pair (`SS/src/lib.rs:303-312`) | How valid time is stored once known (slice 5 dates).<br>"As of" semantics.<br>Known time of retractions and confirmations.<br>Intervals per claim or per support.<br>The definition of "comparable" |
| **A4 Identity and merges** | One identity, many aliases; "A merge is itself a claim, so it can be undone"; exact ids auto-merge, fuzzy → Ask, names never (`knowledge.md:11-13`).<br>"One exact canonical identifier is one entity, across sources and scopes" (`knowledge.md:47`).<br>Nothing typed Person (`vocabulary.md:35`).<br>Rename and transfer are accepted limits (`knowledge.md:55`) | `entity_id = H(identifier)`, with the type outside the id.<br>`entity.identifier` UNIQUE.<br>The alias table allows one alias string on several entities.<br>Type is immutable on main (clash → reject) and mutable on T5 (through the `type` claim).<br>The alias kind is only `canonical` on main | How merges are represented and undone, and the surviving canonical id.<br>Account ↔ Person link (slice 4).<br>Alias collisions.<br>Whether the type is a claim or an attribute |
| **A5 Vocabulary and schema** | Core meta-model / standard / extension layers (`vocabulary.md:8-12`).<br>Namespaced, additive, subtyping allowed (`:15-17`).<br>Each predicate declares domain, range, freshness, authority, sensitivity (`:18`).<br>Definition fields (`:19-25`).<br>Versioned means replay (`:26`).<br>Overlaps resolved by a confirmed mapping claim (`:27`).<br>Cardinality is per subject and observation context, never rejected (`:36`) | Main: one vocabulary per extractor, own namespace only (`extract.rs:99-127`), with no subtyping and no imports.<br>Core meta-model terms (Person, Owner…) are absent from code.<br>T4: `cardinality` one/many.<br>T5: required definition fields, `freshness`/`authority` enums with one value each, alias kinds, and the `type`/`alias` meta-predicates (not in any ADR).<br>The claim id excludes the vocabulary version | Meta-model in code.<br>Subtyping and cross-namespace dependencies.<br>Mapping claims.<br>A SHACL-like validation layer.<br>A datatype registry.<br>How versioning interacts with ids and replay |
| **A6 Traversal and indexes** | "Text search first, then expand 1–2 links through the graph" (`knowledge.md:29`).<br>`about` is one hop, entity as subject or object (`surfaces.md:56`) | Indexes on `claim(subject)` and `claim(object_entity)` only.<br>No predicate or value index.<br>No recursive CTE anywhere.<br>No FTS over entities, aliases or claims | 2-hop or path queries.<br>Graph indexes.<br>Expansion from search hits.<br>Ranking |
| **A7 Store and engine** | SQLite + FTS5 behind a storage trait; a graph DB was considered and rejected "for multi-hop queries we don't need yet" (`adr/0002:8-11`).<br>The projection lives in the same DB (`adr/0009:8`).<br>Research leaned toward staying on SQLite (`research/knowledge-graphs-2026-11.md:6,29`) | All access goes through the `Storage` and `ReadStorage` traits (`CORE/src/lib.rs:337`; `CORE/src/read_only.rs:57-80`).<br>T4's "generation" is TEMP tables plus copy, not a pointer swap | Whether to add an embedded graph or RDF engine, or a hybrid |
| **A8 Several extractors** | Authority defaults per kind, owner override (`knowledge.md:20`).<br>Newer comparable observation; ties → `conflicts_with` (`:49`).<br>An extractor is bound by config, one default per connector (`adr/0008:44-45`; `plugins.md:52`) | **One extractor per source** (the `sources.extractor` column).<br>The claim id excludes the extractor, so equal claims from two extractors merge into one claim with several supports.<br>The checkpoint is per extractor *version*: a version bump re-reads the pair.<br>Old-version supports are kept and never retired (main and T4) | Binding several extractors to one source.<br>Authority across extractors.<br>Retiring an old extractor version's supports.<br>The "plugin authority" field (T5 has a single enum value) |
| **A9 Hypothesis and confirmed** | Statuses `observed/hypothesis/confirmed/retracted/superseded` (`knowledge.md:15`; `GLOSSARY.md:16`).<br>A retraction carries through (`knowledge.md:21-24`).<br>"Evidence, confirmations, rules and outcomes are the only precious inputs" (`knowledge.md:26`).<br>Act needs observed or confirmed (`levels.md:10`).<br>Hypotheses wait for slice 5 (`knowledge.md:46`); correction for slice 7 | `CHECK status IN ('observed','superseded')`.<br>Non-deterministic rules are rejected (`claims.rs:157-159`).<br>No table for confirmations | Where confirmations live, given they are precious and so outside the rebuildable projection.<br>The hypothesis acceptance rule (`adr/0008:55`).<br>The retraction propagation mechanism |
| **A10 Retrieval for agents** | MCP gets `search`, `coverage`, `about`, `why` (`surfaces.md:55`).<br>`about` and `why` signatures (`:56-57`).<br>`--scope` filters every derived field before the limit (`:59`).<br>Results are labelled, not enforced (`:45`).<br>`submit(claim)` over MCP is planned (`surfaces.md:23`) | `why` is CLI only, on the writing open, unscoped and unbounded.<br>No `about`.<br>The MCP startup runs one writing open (`crates/rastro-cli/src/mcp.rs:124-130`) | Search plus graph expansion.<br>An entity lookup or search operation.<br>A query surface beyond `about`/`why` |
| **A11 T4/T5** | Paused; "built, pushed, not merged, awaiting the knowledge-graph discussion" (plan lines 133-141) | §3 above: T4 has no review; T5 fails converge; the merge doesn't compile | Keep, rework or drop |

Also open per the plan's Pause (line 140): traversal, graph indexes and store, multiple extractors on one entity, hypothesis and confirmed claims, merges as claims, a general vocabulary model beyond `code`.

---

## 6. Implicit constraints (invariants a design must keep)

| # | Invariant | Source |
|---|---|---|
| 1 | Evidence is insert-only, enforced by triggers. Forgetting is the only removal and leaves a marker | `initial.sql:28-31`; `evidence_insert_guard.sql`; `knowledge.md:7-9`; `adr/0002:16` |
| 2 | Identical re-observation adds no row; a change appends; A→B→A gives 3 rows | `adr/0005:10-13`; `SS/src/lib.rs:271-301` |
| 3 | Knowledge = f(evidence, confirmations, rules) and is rebuildable; swapping an extractor means replay, not migration | `knowledge.md:26`; `vocabulary.md:26`; `adr/0009:8,33,45` |
| 4 | Projections (FTS, claims) are disposable and never the only copy of anything | `adr/0002:18`; `adr/0009:8` |
| 5 | Rebuild is atomic: a reader sees the old or the new generation, never a mix; a failed rebuild keeps the old one | `adr/0009:19,33`; plan S19 |
| 6 | Content-addressed, versioned ids: the same input gives the same entity and claim id across runs and rebuilds; a revision change means the visible state changed | `adr/0009:23-25`; `claims.rs:74-128`; `claims.rs:280-292` (golden ids) |
| 7 | Renderers target claim id + revision | `decisions/renderers.md:13` |
| 8 | Replay order is (`captured_at`, evidence id), monotonic per (source, scope) | `adr/0009:26`; `SS/src/lib.rs:303-312` |
| 9 | The read path never writes: no migrations, no recovery, no file creation; one snapshot per call | `adr/0002:25-33`; `specs/surfaces.md` R14, R17; `adr/0009:8` |
| 10 | Scope filtering covers metadata (aliases, types, status, revision, links, counts, extraction state), computed from visible supports before the limit; a hidden replacement never hides a visible claim | `adr/0009:27`; `surfaces.md:59`; plan D18, S28 |
| 11 | Plugins request, the core decides: every claim is validated (delivered evidence, `structured`, span inside, a deterministic rule, vocabulary domain and range) | `plugins.md:6-9`; `adr/0008:26-34` |
| 12 | Nothing is derived from a title or body (untrusted content) | `knowledge.md:46`; `adr/0008:34`; plan D19 |
| 13 | Absence is knowledge; an empty answer states coverage and extraction state | `knowledge.md:25,36,52` |
| 14 | A missing or null field, an unreadable item, or an item not returned never closes a support | `knowledge.md:49`; `adr/0009:32` |
| 15 | Conflicts are kept visible, never rejected as cardinality violations | `knowledge.md:19`; `vocabulary.md:36` |
| 16 | Valid time is never fabricated from item-level update times | `knowledge.md:48`; `adr/0009:16,40` |
| 17 | Extractor runs are logged in `runs`, outside the projection, and create no coverage window | `adr/0009:21`; `adr/0008:23`; `plugins.md:55` |
| 18 | A batch's claims and its checkpoint commit in one transaction; a failed run stores nothing | `adr/0009:31`; `extract.rs:244-271` |
| 19 | An extractor reads only its bound sources' evidence; it is never discovered | `adr/0008:44-45`; `plugins.md:52` |
| 20 | Plugins are out of process over JSON-RPC stdio; protocol changes stay additive in 1.x | `adr/0001`; `adr/0003`; `adr/0008:10` |
| 21 | The storage engine sits behind a domain trait in `rastro-core` | `adr/0002:8` |
| 22 | Migrations are named by timestamp, never edited, and a DB with an unknown migration is refused | `adr/0002:20`; `SS/src/lib.rs:115-122` |
| 23 | The concept names no storage engine, language or agent tool | `AGENTS.md` |
| 24 | Two graphs stay apart: execution vs knowledge | `plugins.md:37` |
| 25 | Evidence, confirmations, rules and outcomes export in an open format and rebuild elsewhere | `decisions/sharing.md:6`; `roadmap.md:48` |
| 26 | Shared claims arrive as evidence from a person, not as truth | `sharing.md:8` |
| 27 | Files are private (0700 dirs, 0600 files); the DB lives under the data dir, never in a checkout | `CORE/src/files.rs:7-36`; `adr/0002:19` |

## Unverified
- Whether T4 or T5 compile, or pass tests, on their own today: I didn't run them. Status is taken from the plan, lines 136-137.
- Whether the merged T4+T5 tree has more semantic breaks beyond `claims.rs:288`. I only inspected key files, not a full build.
