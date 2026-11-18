//! Queries over the published, unscoped projection. Never mutates lifecycle state.
use super::*;
use anyhow::ensure;
use rastro_core::knowledge::{
    AboutClaim, AboutClaims, AboutOptions, Citation, ClaimSummary, Explanation, JustificationView,
    Label, LinkReason, LinkReasonKind, LinkedAccount, PersonView, Reconstruction,
    ReconstructionVersions, UnreadSource, WhyOptions,
};
use std::collections::BTreeSet;

/// The store as it stood at a past time, rebuilt in private memory by the same steps a claims
/// rebuild takes: the evidence captured by then is replayed through the lifecycle, then the intents
/// issued by then, the automatic links and the corrections settle on top. Nothing is written to
/// the store, and no stored status, occurrence, link or revision is copied: the answer is today's
/// rules over what existed then, never a memory of what was shown.
struct Past {
    db: Connection,
    versions: Vec<ReconstructionVersions>,
}
/// Copies the rows `sql` selects from `source` into `table` of `target`.
fn copy(
    source: &Connection,
    target: &Connection,
    table: &str,
    sql: &str,
    arguments: &[&dyn rusqlite::ToSql],
) -> Result<()> {
    let mut rows = source.prepare(sql)?;
    let columns = rows.column_count();
    let placeholders = vec!["?"; columns].join(",");
    let mut insert = target.prepare(&format!("INSERT INTO {table} VALUES ({placeholders})"))?;
    let mut found = rows.query(rusqlite::params_from_iter(arguments.iter().copied()))?;
    while let Some(row) = found.next()? {
        insert.execute(rusqlite::params_from_iter(
            (0..columns)
                .map(|i| row.get_ref(i).map(rusqlite::types::ToSqlOutput::Borrowed))
                .collect::<Result<Vec<_>, _>>()?,
        ))?;
    }
    Ok(())
}
/// The ids a query over `db` selects, as a JSON array to pass to another connection's query.
fn id_list(db: &Connection, sql: &str) -> Result<String> {
    let ids = db
        .prepare(sql)?
        .query_map([], |r| r.get::<_, rusqlite::types::Value>(0))?
        .map(|id| {
            Ok(match id? {
                rusqlite::types::Value::Integer(n) => serde_json::Value::from(n),
                rusqlite::types::Value::Text(text) => serde_json::Value::from(text),
                other => anyhow::bail!("unexpected id {other:?}"),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(serde_json::to_string(&ids)?)
}
/// A justification of `j` whose every span cites evidence captured by `?1`: a bundle is read whole
/// or not at all, as a scoped read does with the evidence it cannot see.
const KNOWN_BY: &str = "EXISTS (SELECT 1 FROM justification_span s WHERE s.justification_id = j.id) \
     AND NOT EXISTS (SELECT 1 FROM justification_span s JOIN evidence e ON e.id = s.evidence_id \
     WHERE s.justification_id = j.id AND e.captured_at > ?1)";
/// Rebuilds what `source` held at `at`. A quote is copied only for the evidence `claim` cites:
/// replaying a lifecycle needs none, and `why` cites one claim's.
///
/// A scoped read passes its scoped replay as `source` and the store as `full`. The scoped replay
/// holds every intent, so what its scope cannot see is cut after the intents are applied: only
/// claims the replay holds stay. Whether a correction still holds at `at` is the store's decision
/// then, carried in as a scoped replay carries it today, since the scope's own occurrences are
/// not the ones the correction was bound to.
fn reconstruct(
    source: &Connection,
    at: chrono::DateTime<chrono::Utc>,
    claim: Option<&str>,
    full: Option<&Connection>,
) -> Result<Past> {
    let moment = at;
    let at = crate::stamp(at);
    let db = Connection::open_in_memory()?;
    db.execute_batch("PRAGMA foreign_keys = OFF;")?;
    db.execute_batch(include_str!("projection_temp.sql"))?;
    db.execute_batch("BEGIN")?;
    db.execute_batch(
        "CREATE TABLE evidence (id INTEGER PRIMARY KEY, source TEXT, scope TEXT, location TEXT, field TEXT, start INTEGER, end INTEGER, quote TEXT, captured_at TEXT, source_updated_at TEXT, trust_class TEXT);
         CREATE TABLE intent (append_seq INTEGER PRIMARY KEY, id TEXT, format_version INTEGER,
         kind TEXT, owner_id TEXT, issuer_context TEXT, issued_at TEXT, domain_key TEXT,
         targets_json TEXT, payload_json TEXT, undo_of TEXT, supersedes TEXT)",
    )?;
    copy(
        source,
        &db,
        "justification",
        &format!("SELECT j.* FROM justification j WHERE {KNOWN_BY}"),
        &[&at],
    )?;
    copy(
        source,
        &db,
        "justification_span",
        &format!(
            "SELECT s.* FROM justification_span s JOIN justification j ON j.id = s.justification_id WHERE {KNOWN_BY}"
        ),
        &[&at],
    )?;
    let claims = id_list(&db, "SELECT DISTINCT claim_id FROM justification")?;
    copy(
        source,
        &db,
        "claim",
        "SELECT * FROM claim WHERE id IN (SELECT value FROM json_each(?1))",
        &[&claims],
    )?;
    let entities = id_list(
        &db,
        "SELECT subject FROM claim UNION SELECT object_entity FROM claim WHERE object_entity IS NOT NULL",
    )?;
    copy(
        source,
        &db,
        "entity",
        "SELECT * FROM entity WHERE id IN (SELECT value FROM json_each(?1))",
        &[&entities],
    )?;
    let evidence = id_list(&db, "SELECT DISTINCT evidence_id FROM justification_span")?;
    copy(
        source,
        &db,
        "evidence",
        "SELECT id, source, scope, location, field, start, end, \
         CASE WHEN ?2 IS NOT NULL AND id IN (SELECT s.evidence_id FROM justification_span s JOIN justification j ON j.id = s.justification_id WHERE j.claim_id = ?2) THEN quote ELSE '' END, \
         captured_at, source_updated_at, trust_class FROM evidence WHERE id IN (SELECT value FROM json_each(?1))",
        &[&evidence, &claim],
    )?;
    copy(
        source,
        &db,
        "intent",
        "SELECT append_seq, id, format_version, kind, owner_id, issuer_context, issued_at, domain_key, targets_json, payload_json, undo_of, supersedes FROM intent WHERE issued_at <= ?1",
        &[&at],
    )?;
    let components: BTreeSet<(String, String)> = db
        .prepare("SELECT DISTINCT subject, predicate FROM claim")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    lifecycle::refresh(&db, &components)?;
    super::identity::apply(&db)?;
    super::automatic::apply(&db)?;
    if let Some(full) = full {
        let seen = id_list(source, "SELECT id FROM claim")?;
        for sql in [
            "DELETE FROM claim_link WHERE from_claim NOT IN (SELECT value FROM json_each(?1)) OR to_claim NOT IN (SELECT value FROM json_each(?1))",
            "DELETE FROM intent_justification WHERE claim_id NOT IN (SELECT value FROM json_each(?1))",
            "DELETE FROM automatic_justification WHERE claim_id NOT IN (SELECT value FROM json_each(?1))",
            "DELETE FROM claim WHERE id NOT IN (SELECT value FROM json_each(?1))",
        ] {
            db.execute(sql, [&seen])?;
        }
        db.execute_batch("DELETE FROM entity WHERE id NOT IN (SELECT subject FROM claim UNION SELECT object_entity FROM claim WHERE object_entity IS NOT NULL)")?;
        lifecycle::carry_holds(&reconstruct(full, moment, None, None)?.db, &db)?;
    }
    lifecycle::settle(&db)?;
    let versions = db
        .prepare("SELECT DISTINCT extractor, extractor_version, vocabulary_version FROM justification ORDER BY 1, 2, 3")?
        .query_map([], |r| {
            Ok(ReconstructionVersions {
                extractor: r.get(0)?,
                extractor_version: r.get(1)?,
                vocabulary_version: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    db.execute_batch("COMMIT")?;
    Ok(Past { db, versions })
}

/// Claims of `rastro:alias` (id, subject, subject identifier, status) matching a canonical alias `<namespace>:<kind>/<value>` in `?1`.
/// Split on the first `:` then the first `/`; kind and value compare exactly (no case folding), and
/// the subject's identifier must start with `<namespace>:`. Anything else matches nothing.
/// No status is tested here: a scoped read selects roots without stored (full-scope) status.
pub(super) const ALIAS_CLAIMS: &str = "SELECT c.id AS claim, c.subject, e.identifier, c.status FROM claim c JOIN entity e ON e.id = c.subject \
 WHERE c.predicate = 'rastro:alias' AND instr(?1, ':') > 1 \
 AND instr(substr(?1, instr(?1, ':') + 1), '/') > 1 \
 AND c.object_datatype = substr(substr(?1, instr(?1, ':') + 1), 1, instr(substr(?1, instr(?1, ':') + 1), '/') - 1) \
 AND c.object_value = substr(substr(?1, instr(?1, ':') + 1), instr(substr(?1, instr(?1, ':') + 1), '/') + 1) \
 AND c.object_value <> '' \
 AND substr(e.identifier, 1, instr(?1, ':')) = substr(?1, 1, instr(?1, ':'))";
/// Lookup counts every status except these; a superseded alias is the historical candidate.
const NOT_LOOKED_UP: &str = "('hypothesis', 'retracted')";
/// Statuses that mark a candidate (and an alias shown on a claim) as current.
pub(crate) const CURRENT: &str = "('observed', 'confirmed')";

/// The commitments accepted for this Person's side: the entities an `accept` or `edit` intent
/// minted whose recorded parties include the Person, an account it holds, or a Person merged with
/// it. Their standard statements belong to the Person's view, so `about me` shows what the owner
/// accepted. A scoped replay's intent log holds every intent, so the claims themselves — copied
/// only where their scope is visible — decide what a scope shows.
fn accepted_commitments(
    db: &rusqlite::Connection,
    class: &BTreeSet<String>,
    accounts: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    use rastro_core::identity::{IntentKind, Targets};
    let mut commitments = BTreeSet::new();
    for intent in super::identity::load(db)? {
        if !matches!(intent.kind, IntentKind::Accept | IntentKind::Edit) {
            continue;
        }
        let Targets::Proposal {
            subject: Some(subject),
            parties,
            ..
        } = &intent.targets
        else {
            continue;
        };
        if (parties.iter()).any(|party| class.contains(&party.id) || accounts.contains(&party.id)) {
            commitments.insert(subject.id.clone());
        }
    }
    Ok(commitments)
}
impl View<'_> {
    fn aliases(&self, entity: &str) -> Result<Vec<String>> {
        let rows: Vec<(String, String, String)> = self.0.prepare(&format!("SELECT DISTINCT c.object_value, c.object_datatype, e.identifier FROM claim c JOIN entity e ON e.id = c.subject WHERE c.subject = ?1 AND c.predicate = 'rastro:alias' AND c.status IN {CURRENT} ORDER BY c.object_value"))?
            .query_map([entity], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect::<Result<_, _>>()?;
        let mut aliases = BTreeSet::new();
        for (value, kind, identifier) in rows {
            match identifier.split_once(':') {
                Some((namespace, _)) if kind == "email" => {
                    aliases.insert(format!("{namespace}:email/{value}"));
                }
                _ => {
                    aliases.insert(value);
                }
            }
        }
        Ok(aliases.into_iter().collect())
    }
    /// The entity's displayed type: the most specific type of the compatible family whose newest
    /// member is newest; a tie goes to the lowest claim id.
    fn displayed_type(&self, entity: &str) -> Result<String> {
        let mut types: Vec<(String, String, BTreeSet<String>, String)> = vec![];
        let mut statement = self.0.prepare_cached(&format!("SELECT id, object_value, type_ancestors, known_since FROM claim WHERE subject = ?1 AND predicate = 'rastro:type' AND status IN {CURRENT} AND object_value IS NOT NULL"))?;
        let mut rows = statement.query([entity])?;
        while let Some(row) = rows.next()? {
            types.push((
                row.get(0)?,
                row.get(1)?,
                serde_json::from_str(&row.get::<_, String>(2)?)?,
                row.get(3)?,
            ));
        }
        // Each most specific type (one no other live type descends from) heads a family: itself
        // plus the live ancestors it carries. A family ranks by its newest member's known time,
        // ties to the lowest id of its head. Branching subtypes of one supertype are separate
        // families that each count the shared ancestor's time, so no family needs a single head.
        let family = |ancestors: &BTreeSet<String>, since: &String| {
            types
                .iter()
                .filter(|(_, value, _, _)| ancestors.contains(value))
                .map(|t| &t.3)
                .chain([since])
                .max()
                .cloned()
                .unwrap_or_default()
        };
        Ok(types
            .iter()
            .filter(|(_, value, _, _)| {
                !types
                    .iter()
                    .any(|(_, _, ancestors, _)| ancestors.contains(value))
            })
            .map(|(id, value, ancestors, since)| (family(ancestors, since), id, value))
            .max_by(|a, b| (&a.0, std::cmp::Reverse(a.1)).cmp(&(&b.0, std::cmp::Reverse(b.1))))
            .map(|(_, _, value)| value.clone())
            .unwrap_or_default())
    }
    fn entity(&self, id: &str) -> Result<Entity> {
        let identifier: String =
            self.0
                .query_row("SELECT identifier FROM entity WHERE id = ?1", [id], |row| {
                    row.get(0)
                })?;
        Ok(Entity::new(identifier, self.displayed_type(id)?))
    }
    fn summary(&self, id: &str) -> Result<Option<ClaimSummary>> {
        let row = self.0.query_row("SELECT subject, predicate, object_entity, object_value, object_datatype, status, revision, (SELECT count(*) FROM justification WHERE claim_id = c.id AND current = 1) + (SELECT count(*) FROM intent_justification WHERE claim_id = c.id AND ended_at IS NULL) + (SELECT count(*) FROM automatic_justification WHERE claim_id = c.id AND ended_at IS NULL) FROM claim c WHERE id = ?1", [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, i64>(7)?))).optional()?;
        let Some((subject, predicate, object_entity, value, datatype, status, revision, count)) =
            row
        else {
            return Ok(None);
        };
        let subject = self.entity(&subject)?;
        let object = if let Some(id) = object_entity {
            let Entity {
                id,
                identifier,
                entity_type,
            } = self.entity(&id)?;
            Object::Entity {
                id,
                identifier,
                entity_type,
            }
        } else {
            Object::Literal {
                value: value.ok_or_else(|| anyhow::anyhow!("claim has no literal value"))?,
                datatype: datatype
                    .ok_or_else(|| anyhow::anyhow!("claim has no literal datatype"))?,
            }
        };
        let standing = lifecycle::standing(self.0, id)?;
        let subject_aliases = self.aliases(&subject.id)?;
        let object_aliases = match &object {
            Object::Entity { id, .. } => self.aliases(id)?,
            _ => vec![],
        };
        Ok(Some(ClaimSummary {
            claim: ClaimView {
                id: id.into(),
                subject,
                predicate,
                object,
                status,
                revision,
                known_intervals: lifecycle::intervals(self.0, id)?,
                valid_time: rastro_core::claims::ValidTime::Unknown,
                conflicts_with: lifecycle::links(self.0, id, "conflicts_with", false)?,
                supersedes: lifecycle::links(self.0, id, "supersedes", false)?,
                superseded_by: lifecycle::links(self.0, id, "supersedes", true)?,
                earlier_retracted: standing.is_some_and(|s| !s.confirms && !s.holds),
                stale: standing.is_some_and(|s| s.confirms && !s.holds),
            },
            subject_aliases,
            object_aliases,
            subject_label: None,
            object_label: None,
            support_count: count as usize,
        }))
    }
    /// The entity's label: its title or name, cited, with its current state (knowledge §69). The
    /// text is the latest observation at the entity's own location that `scopes` can see (empty:
    /// every scope), by `captured_at` and then evidence id; hidden evidence never makes one.
    pub(super) fn label(&self, entity: &Entity, scopes: &[Scope]) -> Result<Option<Label>> {
        let Some(field) = rastro_core::knowledge::label_field(&entity.entity_type) else {
            return Ok(None);
        };
        let found = self
            .0
            .query_row(
                "SELECT id, quote, captured_at, trust_class FROM evidence \
                 WHERE location = ?1 AND field = ?2 \
                 AND (?3 IS NULL OR scope IN (SELECT value FROM json_each(?3))) \
                 ORDER BY captured_at DESC, id DESC LIMIT 1",
                params![entity.identifier, field, scope_parameter(scopes)?],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((evidence_id, quote, captured, trust)) = found else {
            return Ok(None);
        };
        let (text, clipped) = rastro_core::knowledge::clip_label(&quote);
        let url = (entity.identifier.starts_with("https://")
            || entity.identifier.starts_with("http://"))
        .then(|| entity.identifier.clone());
        Ok(Some(Label {
            text,
            url,
            evidence_id,
            field: field.into(),
            captured_at: parse_time(captured)?,
            trust_class: trust_class(&trust)?,
            clipped,
            state: self.state(&entity.id, scopes)?,
        }))
    }
    /// The value of the entity's current observed `code:state` or `tracker:state` claim. A scoped
    /// read considers only the claims it can see in full, and takes the one whose newest visible
    /// observation is the latest: stored status is full-scope, and a hidden observation must not
    /// decide what a scope sees.
    fn state(&self, entity: &str, scopes: &[Scope]) -> Result<Option<String>> {
        const STATES: &str = "c.subject = ?1 AND c.predicate IN ('code:state', 'tracker:state') AND c.object_value IS NOT NULL";
        Ok(match scope_parameter(scopes)? {
            None => self.0.query_row(
                &format!("SELECT c.object_value FROM claim c WHERE {STATES} AND c.status IN ('observed', 'confirmed') ORDER BY c.known_since DESC, c.id LIMIT 1"),
                [entity],
                |row| row.get(0),
            ),
            Some(scopes) => self.0.query_row(
                &format!("SELECT c.object_value FROM claim c WHERE {STATES} AND {} \
                    ORDER BY (SELECT max(e.captured_at) FROM justification j \
                        JOIN justification_span s ON s.justification_id = j.id \
                        JOIN evidence e ON e.id = s.evidence_id WHERE j.claim_id = c.id) DESC, c.id LIMIT 1",
                    scoped::VISIBLE_CLAIM),
                params![entity, scopes],
                |row| row.get(0),
            ),
        }
        .optional()?)
    }
    /// The entity id a form names: `me`, an exact id or identifier, else a canonical alias.
    pub(super) fn resolve(&self, entity: &str) -> Result<Option<String>> {
        if entity == rastro_core::identity::ME {
            return super::identity::me(self.0);
        }
        let exact: Option<String> = self.0.query_row(
            "SELECT id FROM entity WHERE id = ?1 OR identifier = ?1 ORDER BY (id = ?1) DESC LIMIT 1", [entity], |row| row.get(0)).optional()?;
        Ok(match exact {
            Some(id) => Some(id),
            None => {
                let candidates = self.0.prepare(&format!("SELECT s.subject, s.identifier, max(s.status IN {CURRENT}) FROM ({ALIAS_CLAIMS}) s WHERE s.status NOT IN {NOT_LOOKED_UP} GROUP BY s.subject ORDER BY s.subject"))?
                    .query_map([entity], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
                if candidates.len() > 1 {
                    bail!(
                        "ambiguous alias {entity}; candidates: {}",
                        candidates
                            .iter()
                            .map(|(id, identifier, current)| format!(
                                "{id} ({identifier}): {}",
                                if *current { "current" } else { "historical" }
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                candidates.into_iter().next().map(|(id, _, _)| id)
            }
        })
    }
    /// The Persons merged with `entity` through any number of observed `rastro:same_as` claims,
    /// itself included.
    fn merge_class(&self, entity: &str) -> Result<BTreeSet<String>> {
        let mut class = BTreeSet::from([entity.to_string()]);
        let mut pending = vec![entity.to_string()];
        let mut statement = self.0.prepare_cached(
            "SELECT subject, object_entity FROM claim WHERE predicate = ?2 AND status IN ('observed', 'confirmed') \
             AND (subject = ?1 OR object_entity = ?1)",
        )?;
        while let Some(next) = pending.pop() {
            let pairs = statement
                .query_map(params![next, rastro_core::identity::SAME_AS], |row| {
                    Ok([row.get::<_, String>(0)?, row.get::<_, String>(1)?])
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for other in pairs.into_iter().flatten() {
                if class.insert(other.clone()) {
                    pending.push(other);
                }
            }
        }
        Ok(class)
    }
    /// Whether the entity is a Person: an owner intent types it `rastro:Person`.
    fn is_person(&self, entity: &str) -> Result<bool> {
        Ok(self.0.query_row(
            "SELECT EXISTS (SELECT 1 FROM claim c WHERE c.subject = ?1 AND c.predicate = 'rastro:type' \
             AND c.object_value = ?2 AND EXISTS (SELECT 1 FROM intent_justification ij WHERE ij.claim_id = c.id))",
            params![entity, rastro_core::identity::PERSON_TYPE],
            |r| r.get(0),
        )?)
    }
    /// What `about` reads. An entity that isn't a Person gives its one-hop claims, up to one more
    /// than `limit`. A Person gives its view: the claims of its merge class and of every account
    /// the class holds through an observed `rastro:held_by`, each account's claims naming it, all
    /// of them read so equivalents can group; their justification spans count against `budget`,
    /// and a view past it fails instead of answering partially.
    #[expect(
        clippy::too_many_lines,
        reason = "the view gathers classes, accounts, claims and their budgets in one pass; the budget must see every claim before it answers"
    )]
    fn about_now(
        &self,
        entity: &str,
        options: &AboutOptions,
        budget: usize,
    ) -> Result<AboutClaims> {
        let Some(entity) = self.resolve(entity)? else {
            return Ok(AboutClaims::default());
        };
        let since = options.since.map(crate::stamp);
        // In a Person's view a claim names the linked account it is about, and every
        // `rastro:held_by` names its account, a superseded one included.
        let summaries = |ids: Vec<String>, accounts: Option<&BTreeSet<String>>| {
            ids.into_iter()
                .map(|id| {
                    let summary = self
                        .summary(&id)?
                        .ok_or_else(|| anyhow::anyhow!("claim missing in snapshot"))?;
                    let object = match &summary.claim.object {
                        Object::Entity { id, .. } => Some(id),
                        Object::Literal { .. } => None,
                    };
                    let Some(accounts) = accounts else {
                        return Ok(AboutClaim {
                            summary,
                            account: None,
                        });
                    };
                    let link = summary.claim.predicate == rastro_core::identity::HELD_BY;
                    let account = [Some(&summary.claim.subject.id), object]
                        .into_iter()
                        .flatten()
                        .find(|e| accounts.contains(*e))
                        .or(link.then_some(&summary.claim.subject.id))
                        .cloned();
                    Ok(AboutClaim { summary, account })
                })
                .collect::<Result<Vec<_>>>()
        };
        if !self.is_person(&entity)? {
            let mut statement = self.0.prepare(
                "SELECT id FROM claim WHERE (subject = ?1 OR object_entity = ?1) \
                 AND (?2 OR status IN ('observed', 'confirmed')) AND (?3 IS NULL OR known_since >= ?3) \
                 ORDER BY known_since DESC, id LIMIT ?4",
            )?;
            let ids = statement
                .query_map(
                    params![
                        entity,
                        options.history,
                        since,
                        // A state filter applies after the read, so it reads every claim.
                        if options.state.is_empty() {
                            (options.limit + 1) as i64
                        } else {
                            -1
                        }
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(AboutClaims {
                resolved: true,
                claims: summaries(ids, None)?,
                person: None,
                reconstruction: None,
            });
        }
        let class = self.merge_class(&entity)?;
        let accounts: BTreeSet<String> = self
            .0
            .prepare(
                "SELECT DISTINCT subject FROM claim WHERE predicate = ?2 AND status IN ('observed', 'confirmed') \
                 AND object_entity IN (SELECT value FROM json_each(?1))",
            )?
            .query_map(
                params![
                    serde_json::to_string(&class)?,
                    rastro_core::identity::HELD_BY
                ],
                |r| r.get(0),
            )?
            .collect::<Result<_, _>>()?;
        let commitments = accepted_commitments(self.0, &class, &accounts)?;
        let view: BTreeSet<&String> = class.iter().chain(&accounts).chain(&commitments).collect();
        let view = serde_json::to_string(&view)?;
        const TOUCHING: &str = "SELECT id FROM claim WHERE subject IN (SELECT value FROM json_each(?1)) \
             UNION SELECT id FROM claim WHERE object_entity IN (SELECT value FROM json_each(?1))";
        let work: i64 = self.0.query_row(
            &format!(
                "SELECT count(*) FROM (SELECT 1 FROM justification_span s JOIN justification j \
                 ON j.id = s.justification_id WHERE j.claim_id IN ({TOUCHING}) LIMIT ?2)"
            ),
            params![view, budget.saturating_add(1) as i64],
            |r| r.get(0),
        )?;
        ensure!(work as usize <= budget, crate::scoped::OVER_BUDGET);
        let ids = self
            .0
            .prepare(&format!(
                "SELECT c.id FROM claim c WHERE c.id IN ({TOUCHING}) \
                 AND (?2 OR c.status IN ('observed', 'confirmed')) AND (?3 IS NULL OR c.known_since >= ?3) \
                 ORDER BY c.known_since DESC, c.id"
            ))?
            .query_map(params![view, options.history, since], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let claims = summaries(ids, Some(&accounts))?;
        let mut linked = Vec::new();
        for account in &accounts {
            let Entity {
                id,
                identifier,
                entity_type,
            } = self.entity(account)?;
            // A link's reasons: each owner intent that still holds, then each automatic justification (token owner, verified email).
            let mut reasons: Vec<LinkReason> = self
                .0
                .prepare(
                    "SELECT c.id, ij.id, ij.intent_id FROM claim c JOIN intent_justification ij ON ij.claim_id = c.id \
                     JOIN intent i ON i.id = ij.intent_id \
                     WHERE i.kind <> 'confirm' AND c.subject = ?1 AND c.predicate = ?3 AND c.status IN ('observed', 'confirmed') AND ij.ended_at IS NULL \
                     AND c.object_entity IN (SELECT value FROM json_each(?2)) ORDER BY c.id, ij.id",
                )?
                .query_map(
                    params![
                        account,
                        serde_json::to_string(&class)?,
                        rastro_core::identity::HELD_BY
                    ],
                    |r| {
                        Ok(LinkReason {
                            kind: LinkReasonKind::OwnerIntent,
                            claim: r.get(0)?,
                            justification: r.get(1)?,
                            intent: Some(r.get(2)?),
                        })
                    },
                )?
                .collect::<Result<_, _>>()?;
            let auto = self.0.prepare("SELECT c.id, a.id, a.reason FROM claim c JOIN automatic_justification a ON a.claim_id = c.id WHERE c.subject = ?1 AND c.predicate = ?3 AND c.status IN ('observed', 'confirmed') AND a.ended_at IS NULL AND c.object_entity IN (SELECT value FROM json_each(?2)) ORDER BY c.id, a.id")?
                .query_map(params![account, serde_json::to_string(&class)?, rastro_core::identity::HELD_BY], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            reasons.extend(
                auto.into_iter()
                    .map(|(claim, justification, reason)| LinkReason {
                        kind: if reason == "token_owner" {
                            LinkReasonKind::TokenOwner
                        } else {
                            LinkReasonKind::VerifiedEmail
                        },
                        claim,
                        justification,
                        intent: None,
                    }),
            );
            linked.push(LinkedAccount {
                aliases: self.aliases(&id)?,
                id,
                identifier,
                entity_type,
                reasons,
            });
        }
        linked.sort_by(|a, b| a.identifier.cmp(&b.identifier));
        let person = self.entity(&entity)?;
        Ok(AboutClaims {
            resolved: true,
            claims,
            person: Some(PersonView {
                id: person.id,
                identifier: person.identifier,
                accounts: linked,
            }),
            reconstruction: None,
        })
    }

    /// What `about` reads: today's claims, or as of `options.as_of` the claims of the store as it
    /// stood then, with what that reconstruction says about itself.
    pub(super) fn about_view(
        &self,
        entity: &str,
        options: &AboutOptions,
        budget: usize,
    ) -> Result<AboutClaims> {
        self.about_past(entity, options, budget, self.0)
    }
    /// [`View::about_view`] on a scoped replay of the store `full`.
    pub(super) fn about_view_scoped(
        &self,
        entity: &str,
        options: &AboutOptions,
        budget: usize,
        full: &Connection,
    ) -> Result<AboutClaims> {
        self.about_past(entity, options, budget, full)
    }
    fn about_past(
        &self,
        entity: &str,
        options: &AboutOptions,
        budget: usize,
        full: &Connection,
    ) -> Result<AboutClaims> {
        let Some(at) = options.as_of else {
            return self.about_now(entity, options, budget);
        };
        // The sources the entity has claims from are today's: a source unread at `at` has no
        // claim in the rebuilt store to show for it.
        let unread = match self.resolve(entity)? {
            Some(_) => {
                let today = AboutOptions {
                    since: None,
                    limit: 1_000_000,
                    history: true,
                    state: Vec::new(),
                    as_of: None,
                };
                let read = self.about_now(entity, &today, budget)?;
                let ids: Vec<String> = read
                    .claims
                    .iter()
                    .map(|c| c.summary.claim.id.clone())
                    .collect();
                self.unread(&ids, at, full)?
            }
            None => Vec::new(),
        };
        let scoped = !std::ptr::eq(self.0, full);
        let past = reconstruct(self.0, at, None, scoped.then_some(full))?;
        let mut read = View(&past.db).about_now(
            entity,
            &AboutOptions {
                as_of: None,
                ..options.clone()
            },
            budget,
        )?;
        read.reconstruction = Some(Reconstruction::new(at, past.versions, unread));
        Ok(read)
    }
    /// The sources, with their scopes, that `claims` cite here today and that held no evidence by
    /// `at` in the store `full`.
    fn unread(
        &self,
        claims: &[String],
        at: chrono::DateTime<chrono::Utc>,
        full: &Connection,
    ) -> Result<Vec<UnreadSource>> {
        let cited: Vec<(String, String)> = self
            .0
            .prepare(
                "SELECT DISTINCT e.source, e.scope FROM justification j \
                 JOIN justification_span s ON s.justification_id = j.id JOIN evidence e ON e.id = s.evidence_id \
                 WHERE j.claim_id IN (SELECT value FROM json_each(?1))",
            )?
            .query_map([serde_json::to_string(claims)?], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        let mut read = full.prepare(
            "SELECT EXISTS (SELECT 1 FROM evidence WHERE source = ?1 AND scope = ?2 AND captured_at <= ?3)",
        )?;
        let mut unread = Vec::new();
        for (source, scope) in cited {
            if !read.query_row(params![source, scope, crate::stamp(at)], |r| {
                r.get::<_, bool>(0)
            })? {
                unread.push(UnreadSource {
                    source,
                    scope: Scope(scope),
                });
            }
        }
        unread.sort_by(|a, b| (&a.source, &a.scope.0).cmp(&(&b.source, &b.scope.0)));
        Ok(unread)
    }
    /// Why a claim is held: today's justifications, or as of `options.as_of` those of the store as
    /// it stood then. A claim that exists today but nothing yet held then is refused as not known
    /// yet, by an error that says what the reconstruction says, so it is not taken for a mistyped
    /// id (`None`).
    pub(super) fn explanation(
        &self,
        claim: &str,
        options: &WhyOptions,
    ) -> Result<Option<Explanation>> {
        self.explanation_past(claim, options, self.0)
    }
    /// [`View::explanation`] on a scoped replay of the store `full`.
    pub(super) fn explanation_scoped(
        &self,
        claim: &str,
        options: &WhyOptions,
        full: &Connection,
    ) -> Result<Option<Explanation>> {
        self.explanation_past(claim, options, full)
    }
    fn explanation_past(
        &self,
        claim: &str,
        options: &WhyOptions,
        full: &Connection,
    ) -> Result<Option<Explanation>> {
        let Some(at) = options.as_of else {
            return self.explanation_now(claim, options);
        };
        let unread = self.unread(&[claim.to_string()], at, full)?;
        let scoped = !std::ptr::eq(self.0, full);
        let past = reconstruct(self.0, at, Some(claim), scoped.then_some(full))?;
        let why = View(&past.db).explanation_now(
            claim,
            &WhyOptions {
                as_of: None,
                ..*options
            },
        )?;
        let reconstruction = Reconstruction::new(at, past.versions, unread);
        let Some(mut why) = why else {
            if self.summary(claim)?.is_none() {
                return Ok(None);
            }
            let unread: String = reconstruction
                .not_read_yet
                .iter()
                .map(|u| {
                    format!(
                        " {}: not read yet at this time (scope {}).",
                        u.source, u.scope.0
                    )
                })
                .collect();
            bail!(
                "claim {claim} is not known as of {}: nothing that holds it had been captured yet. {}{unread}",
                at.to_rfc3339(),
                reconstruction.statement
            );
        };
        why.reconstruction = Some(reconstruction);
        Ok(Some(why))
    }
    fn explanation_now(&self, claim: &str, options: &WhyOptions) -> Result<Option<Explanation>> {
        let Some(claim) = self.summary(claim)? else {
            return Ok(None);
        };
        let mut statement = self.0.prepare("SELECT id, kind, extractor, extractor_version, extraction_rule, vocabulary_version, current FROM justification WHERE claim_id = ?1 ORDER BY current DESC, id LIMIT ?2")?;
        let mut supports = statement
            .query_map(params![claim.claim.id, (options.limit + 1) as i64], |row| {
                Ok(JustificationView {
                    justification_id: row.get(0)?,
                    justification_kind: row.get(1)?,
                    extractor: row.get(2)?,
                    extractor_version: row.get(3)?,
                    extraction_rule: row.get(4)?,
                    vocabulary_version: row.get(5)?,
                    current: row.get(6)?,
                    spans: vec![],
                    intent: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        // An owner intent's justification: current while the intent stands, not once it was
        // undone or its name replaced.
        let mut statement = self.0.prepare("SELECT ij.id, i.id, i.kind, i.format_version, i.issued_at, i.issuer_context, ij.ended_at IS NULL FROM intent_justification ij JOIN intent i ON i.id = ij.intent_id WHERE ij.claim_id = ?1 ORDER BY ij.ended_at IS NULL DESC, ij.id LIMIT ?2")?;
        let rows = statement
            .query_map(params![claim.claim.id, (options.limit + 1) as i64], |row| {
                Ok((
                    [row.get::<_, String>(0)?, row.get(1)?, row.get(2)?],
                    row.get::<_, i64>(3)?,
                    [row.get::<_, String>(4)?, row.get(5)?],
                    row.get::<_, bool>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for ([id, intent, kind], version, [issued_at, issuer], current) in rows {
            supports.push(JustificationView {
                justification_id: id,
                // A confirmation is the owner's word on a claim, not a claim the owner made.
                justification_kind: if kind == "confirm" {
                    "confirmation".into()
                } else {
                    rastro_core::identity::OWNER_INTENT.into()
                },
                extractor: rastro_core::identity::OWNER.into(),
                extractor_version: version.to_string(),
                extraction_rule: kind,
                vocabulary_version: String::new(),
                current,
                spans: vec![],
                intent: Some(rastro_core::knowledge::IntentCitation {
                    id: intent,
                    issued_at: parse_time(issued_at)?,
                    issuer: serde_json::from_str(&issuer)?,
                }),
            });
        }
        let mut statement = self.0.prepare("SELECT id, reason, ended_at IS NULL FROM automatic_justification WHERE claim_id = ?1 ORDER BY ended_at IS NULL DESC, id LIMIT ?2")?;
        let automatic = statement
            .query_map(params![claim.claim.id, (options.limit + 1) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (id, reason, current) in automatic {
            supports.push(JustificationView {
                justification_id: id,
                justification_kind: "inference".into(),
                extractor: "rastro".into(),
                extractor_version: String::new(),
                extraction_rule: reason,
                vocabulary_version: String::new(),
                current,
                spans: vec![],
                intent: None,
            });
        }
        supports.sort_by(|a, b| {
            (!a.current, &a.justification_id).cmp(&(!b.current, &b.justification_id))
        });
        let truncated = supports.len() > options.limit;
        supports.truncate(options.limit);
        for support in &mut supports {
            if support.justification_kind == "inference" {
                let premise_ids: Vec<String> = self.0.prepare("SELECT j.id FROM automatic_justification a JOIN justification j ON j.claim_id = a.premise_a OR j.claim_id = a.premise_b WHERE a.id = ?1 ORDER BY j.id")?
                    .query_map([&support.justification_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
                for id in premise_ids {
                    support.spans.extend(self.citations(&id, options.full)?);
                }
            } else {
                support.spans = self.citations(&support.justification_id, options.full)?;
            }
        }
        Ok(Some(Explanation {
            claim: Box::new(claim),
            supports,
            truncated,
            coverage: vec![],
            extraction: vec![],
            reconstruction: None,
        }))
    }
    fn citations(&self, justification: &str, full: bool) -> Result<Vec<Citation>> {
        let mut statement = self.0.prepare("SELECT e.id, s.portable_key, e.source, e.scope, e.location, s.field, s.start, s.end, e.start, e.quote, e.captured_at, e.source_updated_at, e.trust_class FROM justification_span s JOIN evidence e ON e.id = s.evidence_id WHERE s.justification_id = ?1 ORDER BY e.captured_at, e.id, s.portable_key, s.field, s.start, s.end")?;
        let rows = statement.query_map([justification], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)? as u64,
                row.get::<_, i64>(7)? as u64,
                row.get::<_, i64>(8)? as u64,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, String>(12)?,
            ))
        })?;
        rows.map(|row| {
            let (
                evidence_id,
                portable_key,
                source,
                scope,
                location,
                field,
                start,
                end,
                origin,
                quote,
                captured,
                updated,
                trust,
            ) = row?;
            let span = start
                .checked_sub(origin)
                .zip(end.checked_sub(origin))
                .and_then(|(s, e)| quote.get(s as usize..e as usize))
                .ok_or_else(|| anyhow::anyhow!("stored justification span outside evidence"))?;
            Ok(Citation {
                evidence_id,
                portable_key,
                source,
                scope: Scope(scope),
                location,
                field,
                start,
                end,
                captured_at: parse_time(captured)?,
                source_updated_at: updated.map(parse_time).transpose()?,
                trust_class: trust_class(&trust)?,
                excerpt: rastro_core::knowledge::CitationExcerpt::new(span, start),
                quote: full.then(|| span.into()),
            })
        })
        .collect()
    }
}
