//! Evidence-derived identity links, rebuilt from projected claims and owner intents.
use crate::{identity, lifecycle, stamp};
use anyhow::Result;
use rastro_core::claims::{self, Object, Support, revision_with_links};
use rastro_core::identity::{self as core, Resolution, admit_account};
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

/// The Persons merged with `person`, over every merge whether the scope sees it or not: hidden
/// data may suppress an automatic link (D50).
fn merge_class(resolution: &Resolution, person: &str) -> BTreeSet<String> {
    let mut class = BTreeSet::from([person.to_string()]);
    loop {
        let before = class.len();
        for (a, b) in resolution.merges.keys() {
            if class.contains(a) || class.contains(b) {
                class.insert(a.clone());
                class.insert(b.clone());
            }
        }
        if class.len() == before {
            break;
        }
    }
    class
}

/// A cannot-link between `account` and a Person of `person`'s merge class, or an account an owner
/// link holds in it, visible or not (D48, D50).
fn vetoed(resolution: &Resolution, account: &str, person: &str) -> bool {
    let class = merge_class(resolution, person);
    class
        .iter()
        .any(|p| resolution.cannot.contains_key(&pair(account, p)))
        || resolution.links.keys().any(|(other, p)| {
            class.contains(p) && resolution.cannot.contains_key(&pair(account, other))
        })
}

/// D48: two qualifying automatic links into one merge class, cannot-linked to each other, block
/// each other in every interval; an owner link to the class wins.
fn conflicts(
    resolution: &Resolution,
    qualifying: &BTreeSet<(String, String)>,
) -> BTreeSet<(String, String)> {
    let candidates: Vec<&(String, String)> = qualifying.iter().collect();
    let owners: Vec<&(String, String)> = resolution.links.keys().collect();
    let mut blocked = BTreeSet::new();
    for (i, (a, p)) in candidates.iter().enumerate() {
        let class = merge_class(resolution, p);
        for (b, q) in &candidates[i + 1..] {
            if a == b || !class.contains(q) || !resolution.cannot.contains_key(&pair(a, b)) {
                continue;
            }
            let a_owner = owners
                .iter()
                .any(|(account, person)| account == a && class.contains(person));
            let b_owner = owners
                .iter()
                .any(|(account, person)| account == b && class.contains(person));
            for (side, owned) in [(a, a_owner), (b, b_owner)] {
                if !owned {
                    for person in &class {
                        blocked.insert((side.clone(), person.clone()));
                    }
                }
            }
        }
    }
    blocked
}

pub(crate) fn apply(db: &Connection) -> Result<()> {
    apply_within(db, None)
}

/// The (account, Person) pairs `db` links automatically, in any interval.
pub(crate) fn linked_pairs(db: &Connection) -> Result<BTreeSet<(String, String)>> {
    Ok(db
        .prepare("SELECT DISTINCT c.subject, c.object_entity FROM claim c JOIN automatic_justification a ON a.claim_id = c.id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?)
}

/// In a scoped replay, `allowed` holds the (account, Person) pairs the full projection links
/// automatically: hidden data may suppress a link there but never create one (D50).
pub(crate) fn apply_within(
    db: &Connection,
    allowed: Option<&BTreeSet<(String, String)>>,
) -> Result<()> {
    let resolution = Resolution::of(&identity::load(db)?);
    let mut affected: BTreeSet<String> = db
        .prepare("SELECT DISTINCT claim_id FROM automatic_justification")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    affected.extend(db.prepare("SELECT id FROM claim WHERE predicate = ?1 AND EXISTS (SELECT 1 FROM intent_justification WHERE claim_id = claim.id)")?
        .query_map([core::HELD_BY], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?);
    let mut blocked = BTreeSet::new();
    // Every blocked set tried so far, oldest first.
    let mut tried: Vec<BTreeSet<(String, String)>> = Vec::new();
    let mut settled = false;
    loop {
        let mut qualifying = BTreeSet::new();
        db.execute("DELETE FROM automatic_justification", [])?;
        db.execute("UPDATE claim SET status = 'superseded' WHERE predicate = ?1 AND NOT EXISTS (SELECT 1 FROM intent_justification WHERE claim_id = claim.id AND ended_at IS NULL) AND NOT EXISTS (SELECT 1 FROM justification WHERE claim_id = claim.id AND current = 1)", [core::HELD_BY])?;
        if let Some(me) = resolution.me.as_ref() {
            let person = &resolution.persons[me];
            let created: String = db.query_row(
                "SELECT issued_at FROM intent WHERE id = ?1",
                [&person.created_by],
                |r| r.get(0),
            )?;
            let created = crate::parse_time(created)?;
            let rows: Vec<(String, String, String)> = db.prepare(
            "SELECT c.id, c.subject, e.identifier FROM claim c JOIN entity e ON e.id = c.subject WHERE c.predicate = ?1"
        )?.query_map([core::TOKEN_OWNER], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            for (premise, account, identifier) in rows {
                if vetoed(&resolution, &account, me)
                    || allowed.is_some_and(|a| !a.contains(&(account.clone(), me.clone())))
                    || admit_account(&identifier, &identity::types(db, &account, &[])?).is_err()
                {
                    continue;
                }
                let object = Object::Entity {
                    id: me.clone(),
                    identifier: person.entity.identifier.clone(),
                    entity_type: core::PERSON_TYPE.into(),
                };
                let claim = claims::claim_id(&account, core::HELD_BY, &object);
                for interval in lifecycle::intervals(db, &premise)? {
                    let start = interval.start.max(created);
                    if interval.end.is_some_and(|end| end <= start) {
                        continue;
                    }
                    qualifying.insert((account.clone(), me.clone()));
                    if blocked.contains(&(account.clone(), me.clone())) {
                        continue;
                    }
                    db.execute("INSERT OR IGNORE INTO claim (id, subject, predicate, object_entity, scalar, status, revision) VALUES (?1, ?2, ?3, ?4, 0, 'observed', '')", params![claim, account, core::HELD_BY, me])?;
                    let id = format!("automatic/{claim}/token/{premise}/{}", stamp(start));
                    db.execute("INSERT OR IGNORE INTO automatic_justification (id, claim_id, reason, premise_a, premise_b, started_at, ended_at) VALUES (?1, ?2, 'token_owner', ?3, NULL, ?4, ?5)", params![id, claim, premise, stamp(start), interval.end.map(stamp)])?;
                    if interval.end.is_none() {
                        db.execute(
                            "UPDATE claim SET status = 'observed' WHERE id = ?1",
                            [&claim],
                        )?;
                    }
                    affected.insert(claim.clone());
                }
            }
        }
        let emails: Vec<(String, String, String, String)> = db.prepare(
        "SELECT c.id, c.subject, e.identifier, c.object_value FROM claim c JOIN entity e ON e.id = c.subject WHERE c.predicate = ?1 AND c.object_value IS NOT NULL"
    )?.query_map([core::VERIFIED_EMAIL], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        loop {
            let mut added = false;
            let links: Vec<(String, String, String, String)> = db.prepare(
            "SELECT c.id, c.subject, c.object_entity, e.identifier FROM claim c JOIN entity e ON e.id = c.object_entity WHERE c.predicate = ?1"
        )?.query_map([core::HELD_BY], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
            for (seed, anchor, person, person_identifier) in links {
                for (left_id, left_account, _, left_email) in &emails {
                    if *left_account != anchor {
                        continue;
                    }
                    for (right_id, right_account, identifier, right_email) in &emails {
                        if right_account == left_account
                            || !left_email.eq_ignore_ascii_case(right_email)
                            || vetoed(&resolution, right_account, &person)
                            || allowed.is_some_and(|a| {
                                !a.contains(&(right_account.clone(), person.clone()))
                            })
                            || admit_account(identifier, &identity::types(db, right_account, &[])?)
                                .is_err()
                        {
                            continue;
                        }
                        let object = Object::Entity {
                            id: person.clone(),
                            identifier: person_identifier.clone(),
                            entity_type: core::PERSON_TYPE.into(),
                        };
                        let claim = claims::claim_id(right_account, core::HELD_BY, &object);
                        for a in lifecycle::intervals(db, left_id)? {
                            for b in lifecycle::intervals(db, right_id)? {
                                for held in lifecycle::intervals(db, &seed)? {
                                    let start = a.start.max(b.start).max(held.start);
                                    let end = [a.end, b.end, held.end].into_iter().flatten().min();
                                    if end.is_some_and(|end| end <= start) {
                                        continue;
                                    }
                                    qualifying.insert((right_account.clone(), person.clone()));
                                    if blocked.contains(&(right_account.clone(), person.clone())) {
                                        continue;
                                    }
                                    db.execute("INSERT OR IGNORE INTO claim (id, subject, predicate, object_entity, scalar, status, revision) VALUES (?1, ?2, ?3, ?4, 0, 'observed', '')", params![claim, right_account, core::HELD_BY, person])?;
                                    let id = format!(
                                        "automatic/{claim}/email/{left_id}/{right_id}/{seed}/{}/{}",
                                        stamp(start),
                                        end.map(stamp).unwrap_or_default()
                                    );
                                    let count = db.execute("INSERT OR IGNORE INTO automatic_justification (id, claim_id, reason, premise_a, premise_b, started_at, ended_at) VALUES (?1, ?2, 'verified_email', ?3, ?4, ?5, ?6)", params![id, claim, left_id, right_id, stamp(start), end.map(stamp)])?;
                                    if end.is_none() {
                                        db.execute(
                                            "UPDATE claim SET status = 'observed' WHERE id = ?1",
                                            [&claim],
                                        )?;
                                    }
                                    added |= count != 0;
                                    affected.insert(claim.clone());
                                }
                            }
                        }
                    }
                }
            }
            if !added {
                break;
            }
        }
        if settled {
            break;
        }
        // D48: the blocked set is recomputed from this pass's qualifying links, never accumulated,
        // so a conflict that no longer qualifies stops blocking. Should the sets cycle, every set
        // in the cycle stays blocked and one last pass derives with their union.
        let next = conflicts(&resolution, &qualifying);
        if next == blocked {
            break;
        }
        match tried.iter().position(|set| *set == next) {
            Some(start) => {
                blocked = tried[start..]
                    .iter()
                    .flatten()
                    .chain(blocked.iter())
                    .cloned()
                    .collect();
                settled = true;
            }
            None => tried.push(std::mem::replace(&mut blocked, next)),
        }
    }
    for id in affected {
        let mut keys: Vec<String> = db.prepare("SELECT id FROM automatic_justification WHERE claim_id = ?1 AND ended_at IS NULL UNION SELECT intent_id FROM intent_justification WHERE claim_id = ?1 AND ended_at IS NULL")?
            .query_map([&id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        keys.sort();
        let status = if keys.is_empty() {
            "superseded"
        } else {
            "observed"
        };
        let supports: Vec<Support> = keys
            .into_iter()
            .map(|key| Support {
                portable_key: key,
                evidence_id: 0,
                field: String::new(),
                start: 0,
                end: 0,
                extraction_rule: String::new(),
            })
            .collect();
        let since = lifecycle::intervals(db, &id)?
            .last()
            .map(|i| stamp(i.start))
            .unwrap_or_default();
        db.execute("UPDATE claim SET status = ?2, revision = ?3, known_since = ?4 WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM justification WHERE claim_id = ?1)", params![id, status, revision_with_links(status, &supports, &[]), since])?;
    }
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
    Ok(())
}
