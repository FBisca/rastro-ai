//! Owner intents in storage (ADR 0010): the insert-only `intent` table, the intent-backed claims
//! it gives the projection, and the issuer's visibility check. What an intent means is decided by
//! `rastro_core::identity`; this module stores and replays it.
use crate::{parse_time, stamp};
use anyhow::{Context, Result, ensure};
use rastro_core::Scope;
use rastro_core::claims::{Object, intent_justification_id, revision_with_links};
use rastro_core::identity::{HELD_BY, Intent, IntentKind, Resolution, SAME_AS};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};

const COLUMNS: &str = "append_seq, id, format_version, kind, owner_id, issuer_context, issued_at, \
    domain_key, targets_json, payload_json, undo_of, supersedes";

/// One stored row: the text columns, `append_seq`, `format_version`, `undo_of` and `supersedes`.
type Row = ([String; 8], i64, i64, [Option<String>; 2]);
fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok((
        [
            r.get(1)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
        ],
        r.get(0)?,
        r.get(2)?,
        [r.get(10)?, r.get(11)?],
    ))
}
fn parse(row: Row) -> Result<Intent> {
    let (
        [
            id,
            kind,
            owner_id,
            issuer,
            issued_at,
            domain_key,
            targets,
            payload,
        ],
        append_seq,
        format_version,
        [undo_of, supersedes],
    ) = row;
    Ok(Intent {
        kind: IntentKind::parse(&kind)
            .with_context(|| format!("intent {id} has an unknown kind {kind:?}"))?,
        issuer_context: serde_json::from_str(&issuer)?,
        issued_at: parse_time(issued_at)?,
        targets: serde_json::from_str(&targets)?,
        payload: serde_json::from_str(&payload)?,
        id,
        append_seq,
        format_version,
        owner_id,
        domain_key,
        undo_of,
        supersedes,
    })
}
/// Every accepted intent, in acceptance order.
pub(crate) fn load(db: &Connection) -> Result<Vec<Intent>> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM main.intent ORDER BY append_seq"
    ))?;
    let rows = statement.query_map([], row)?;
    rows.map(|r| parse(r?)).collect()
}
fn find(db: &Connection, id: &str) -> Result<Option<Intent>> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM main.intent WHERE id = ?1"),
        [id],
        row,
    )
    .optional()?
    .map(parse)
    .transpose()
}

/// Checks and stores intents in order. What each may do (`Resolution::admit`) is judged against
/// the intents accepted before it, this batch's included; whether the issuer sees its targets is
/// judged against the state when the batch was issued, so an undo doesn't hide what its own
/// cannot-link names. Returns them with their acceptance order; a retry stores nothing.
pub(crate) fn insert_all(
    db: &Connection,
    sees: &[Scope],
    intents: Vec<Intent>,
) -> Result<Vec<Intent>> {
    let mut log = load(db)?;
    let issued = Resolution::of(&log);
    let mut stored = Vec::new();
    // Entities minted earlier in this batch are new: nobody could see them before.
    let mut minted = BTreeSet::new();
    for mut intent in intents {
        intent.check()?;
        if let Some(existing) = find(db, &intent.id)? {
            ensure!(
                existing.same_as_retry(&intent),
                "intent id {} is already used by a different intent",
                intent.id
            );
            stored.push(existing);
            continue;
        }
        Resolution::of(&log).admit(&intent)?;
        for entity in intent.targets.entities() {
            if intent.kind == IntentKind::CreatePerson || minted.contains(&entity.id) {
                minted.insert(entity.id.clone());
                continue;
            }
            ensure!(
                visible(db, &issued, &entity.id, sees)?,
                "the issuing context can't see a target of this {} intent",
                intent.kind.as_str()
            );
        }
        db.execute(
            "INSERT INTO main.intent (id, format_version, kind, owner_id, issuer_context, issued_at,
             domain_key, targets_json, payload_json, undo_of, supersedes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                intent.id,
                intent.format_version,
                intent.kind.as_str(),
                intent.owner_id,
                serde_json::to_string(&intent.issuer_context)?,
                stamp(intent.issued_at),
                intent.domain_key,
                serde_json::to_string(&intent.targets)?,
                serde_json::to_string(&intent.payload)?,
                intent.undo_of,
                intent.supersedes,
            ],
        )?;
        intent.append_seq = db.last_insert_rowid();
        log.push(intent.clone());
        stored.push(intent);
    }
    Ok(stored)
}

/// Recomputes every intent-backed claim of the projection the connection writes (the active one,
/// or a staged rebuild) from the intent table. A pure function of the intents, so a rebuild and
/// an incremental write give the same rows.
pub(crate) fn apply(db: &Connection) -> Result<()> {
    let resolution = Resolution::of(&load(db)?);
    db.execute("DELETE FROM intent_justification", [])?;
    db.execute(
        &format!(
            "DELETE FROM claim_link WHERE from_claim IN ({0}) OR to_claim IN ({0})",
            crate::UNJUSTIFIED
        ),
        [],
    )?;
    db.execute(
        &format!("DELETE FROM claim WHERE id IN ({})", crate::UNJUSTIFIED),
        [],
    )?;
    // Each claim's intents, and whether each still holds.
    let mut supports = BTreeMap::<String, Vec<(String, bool)>>::new();
    for claim in resolution.claims() {
        let mut entities = vec![(&claim.subject.id, &claim.subject.identifier)];
        let (object_entity, object_value, object_datatype) = match &claim.object {
            Object::Entity { id, identifier, .. } => {
                entities.push((id, identifier));
                (Some(id), None, None)
            }
            Object::Literal { value, datatype } => (None, Some(value), Some(datatype)),
        };
        for (id, identifier) in entities {
            db.execute(
                "INSERT OR IGNORE INTO entity (id, identifier) VALUES (?1, ?2)",
                params![id, identifier],
            )?;
        }
        db.execute(
            "INSERT OR IGNORE INTO claim (id, subject, predicate, object_entity, object_value,
             object_datatype, scalar, status, revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'observed', '')",
            params![
                claim.id,
                claim.subject.id,
                claim.predicate,
                object_entity,
                object_value,
                object_datatype,
                claim.scalar
            ],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO intent_justification (id, claim_id, intent_id, ended_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                intent_justification_id(&claim.id, &claim.intent),
                claim.id,
                claim.intent,
                claim.ended_at.map(stamp)
            ],
        )?;
        supports
            .entry(claim.id)
            .or_default()
            .push((claim.intent, claim.ended_at.is_none()));
    }
    // A claim only intents hold is observed while any of them holds and superseded once none
    // does; its revision hashes the intents that hold, as an evidence claim's hashes its current
    // spans, and it is known since the start of its latest interval.
    let mut update = db.prepare_cached(
        "UPDATE claim SET status = ?2, revision = ?3, known_since = ?4 WHERE id = ?1
         AND NOT EXISTS (SELECT 1 FROM justification j WHERE j.claim_id = claim.id)",
    )?;
    for (id, intents) in supports {
        let keys: Vec<_> = intents
            .iter()
            .filter(|(_, holds)| *holds)
            .map(|(intent, _)| rastro_core::claims::Support {
                portable_key: intent.clone(),
                evidence_id: 0,
                field: String::new(),
                start: 0,
                end: 0,
                extraction_rule: String::new(),
            })
            .collect();
        let status = if keys.is_empty() {
            "superseded"
        } else {
            "observed"
        };
        let since = crate::lifecycle::intervals(db, &id)?
            .last()
            .map(|i| stamp(i.start))
            .unwrap_or_default();
        update.execute(params![
            id,
            status,
            revision_with_links(status, &keys, &[]),
            since
        ])?;
    }
    db.execute("DELETE FROM entity WHERE id NOT IN (SELECT subject FROM claim UNION SELECT object_entity FROM claim WHERE object_entity IS NOT NULL)", [])?;
    Ok(())
}

/// Whether a scope set (empty: every scope) can see an entity. An account is visible where a
/// justification of a claim about it is; a Person where at least one of its linked accounts is,
/// and `me` everywhere (ADR 0010, Visibility).
pub(crate) fn visible(
    db: &Connection,
    resolution: &Resolution,
    id: &str,
    scopes: &[Scope],
) -> Result<bool> {
    if scopes.is_empty() {
        return Ok(true);
    }
    let scopes = serde_json::to_string(scopes)?;
    if let Some(person) = resolution.persons.get(id) {
        if person.me {
            return Ok(true);
        }
        for (account, _) in resolution.links.keys().filter(|(_, p)| p == id) {
            if account_visible(db, account, &scopes)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    account_visible(db, id, &scopes)
}
fn account_visible(db: &Connection, id: &str, scopes: &str) -> Result<bool> {
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM claim c WHERE c.subject = ?1 AND {0})
         OR EXISTS (SELECT 1 FROM claim c WHERE c.object_entity = ?1 AND {0})",
        crate::scoped::VISIBLE_CLAIM
    );
    Ok(db.query_row(&sql, params![id, scopes], |r| r.get(0))?)
}

/// Copies the intent-backed claims a scope set can see, with their intents, into a private
/// scoped projection: a Person's own claims where the Person is visible, a link where its
/// account is, a merge where both Persons are. Endpoint entities are copied by the caller.
pub(crate) fn copy_visible(db: &Connection, target: &Connection, scopes: &[Scope]) -> Result<()> {
    target.execute_batch(
        "CREATE TABLE intent (append_seq INTEGER PRIMARY KEY, id TEXT, format_version INTEGER,
         kind TEXT, owner_id TEXT, issuer_context TEXT, issued_at TEXT, domain_key TEXT,
         targets_json TEXT, payload_json TEXT, undo_of TEXT, supersedes TEXT)",
    )?;
    let intents = load(db)?;
    let resolution = Resolution::of(&intents);
    let scope_set = serde_json::to_string(scopes)?;
    let mut accounts = BTreeMap::<String, bool>::new();
    let mut account = |id: &str| -> Result<bool> {
        if let Some(&seen) = accounts.get(id) {
            return Ok(seen);
        }
        let seen = scopes.is_empty() || account_visible(db, id, &scope_set)?;
        accounts.insert(id.into(), seen);
        Ok(seen)
    };
    let mut persons: BTreeSet<String> = resolution.me.iter().cloned().collect();
    for (a, p) in resolution.links.keys() {
        if account(a)? {
            persons.insert(p.clone());
        }
    }
    let claims: Vec<(String, String, String, Option<String>)> = db
        .prepare(
            "SELECT c.id, c.predicate, c.subject, c.object_entity FROM claim c
             WHERE EXISTS (SELECT 1 FROM intent_justification ij WHERE ij.claim_id = c.id)",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let mut shown = Vec::new();
    for (id, predicate, subject, object) in claims {
        // A link or a merge only where both its ends are: an effective link makes its Person
        // visible, but an undone one doesn't.
        let seen = match (predicate.as_str(), object) {
            (HELD_BY, Some(person)) => account(&subject)? && persons.contains(&person),
            (SAME_AS, Some(other)) => persons.contains(&subject) && persons.contains(&other),
            _ => persons.contains(&subject),
        };
        if seen {
            shown.push(id);
        }
    }
    let shown = serde_json::to_string(&shown)?;
    let mut rows =
        db.prepare("SELECT * FROM claim WHERE id IN (SELECT value FROM json_each(?1))")?;
    let columns = rows.column_count();
    let placeholders = vec!["?"; columns].join(",");
    let mut insert = target.prepare(&format!(
        "INSERT OR IGNORE INTO claim VALUES ({placeholders})"
    ))?;
    let mut found = rows.query([&shown])?;
    while let Some(row) = found.next()? {
        insert.execute(rusqlite::params_from_iter(
            (0..columns)
                .map(|i| row.get_ref(i).map(rusqlite::types::ToSqlOutput::Borrowed))
                .collect::<Result<Vec<_>, _>>()?,
        ))?;
    }
    let justifications: Vec<(String, String, String, Option<String>)> = db
        .prepare("SELECT id, claim_id, intent_id, ended_at FROM intent_justification WHERE claim_id IN (SELECT value FROM json_each(?1))")?
        .query_map([&shown], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    let mut needed: BTreeSet<String> = resolution
        .me
        .as_ref()
        .map(|me| resolution.persons[me].created_by.clone())
        .into_iter()
        .collect();
    for (id, claim, intent, ended_at) in &justifications {
        target.execute(
            "INSERT OR IGNORE INTO intent_justification (id, claim_id, intent_id, ended_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, claim, intent, ended_at],
        )?;
        needed.insert(intent.clone());
    }
    for intent in intents.iter().filter(|i| needed.contains(&i.id)) {
        target.execute(
            &format!("INSERT INTO intent ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"),
            params![
                intent.append_seq,
                intent.id,
                intent.format_version,
                intent.kind.as_str(),
                intent.owner_id,
                serde_json::to_string(&intent.issuer_context)?,
                stamp(intent.issued_at),
                intent.domain_key,
                serde_json::to_string(&intent.targets)?,
                serde_json::to_string(&intent.payload)?,
                intent.undo_of,
                intent.supersedes,
            ],
        )?;
    }
    Ok(())
}

/// The owner's Person, from the intent that minted it.
pub(crate) fn me(db: &Connection) -> Result<Option<String>> {
    let targets: Option<String> = db
        .query_row(
            "SELECT targets_json FROM intent WHERE kind = 'create_person' AND domain_key = ?1
             ORDER BY append_seq LIMIT 1",
            [rastro_core::identity::ME_DOMAIN],
            |r| r.get(0),
        )
        .optional()?;
    let Some(targets) = targets else {
        return Ok(None);
    };
    let targets: rastro_core::identity::Targets = serde_json::from_str(&targets)?;
    Ok(targets.entities().first().map(|e| e.id.clone()))
}

/// The entities a scoped `about` on `form` replays besides the form's own: when the form may
/// name a Person, that Person, the Persons merged with it and the accounts they hold. Chosen
/// from every intent; what the scope set sees of them is decided in the replay.
pub(crate) fn roots(db: &Connection, form: &str) -> Result<BTreeSet<String>> {
    Ok(Resolution::of(&load(db)?).roots(form))
}
