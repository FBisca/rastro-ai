//! Replay affected field groups and their AND-bundle dependencies in append order.
//! Slots are interned once; an event visits only derivations depending on its slots.
use crate::parse_time;
use anyhow::Result;
use rastro_core::claims::{
    KnownInterval, Support, occurrence_id_with_ordinal, revision_with_links,
};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Event = (String, i64);
type Component = (String, String);
pub(crate) type AffectedGroup = (String, String, String, String);
type Field = (String, String);
type Slot = (String, String, String, String, String, Option<String>);
type Updated = chrono::DateTime<chrono::Utc>;
struct Span {
    source: String,
    scope: String,
    location: String,
    event: Event,
    updated: Option<Updated>,
    support: Support,
    slot: usize,
    group: usize,
}
struct Derivation {
    id: String,
    claim: String,
    scalar: bool,
    current: bool,
    spans: Vec<Span>,
    requirements: Vec<Vec<usize>>,
    groups: BTreeSet<usize>,
}
impl Derivation {
    fn active<'a>(&'a self, latest: &'a [Option<Event>]) -> impl Iterator<Item = &'a Span> {
        self.spans
            .iter()
            .filter(|s| latest[s.slot].as_ref() == Some(&s.event))
    }
    fn live(&self, latest: &[Option<Event>]) -> bool {
        self.requirements.iter().all(|alternatives| {
            alternatives.iter().any(|&s| {
                let span = &self.spans[s];
                latest[span.slot].as_ref() == Some(&span.event)
            })
        })
    }
}
#[derive(Default)]
struct Group {
    derivations: BTreeSet<usize>,
    candidates: BTreeSet<usize>,
    blocked: BTreeSet<usize>,
}
struct Replay {
    derivations: Vec<Derivation>,
    fields: BTreeMap<Field, usize>,
    groups: Vec<Group>,
    dependents: Vec<BTreeSet<usize>>,
    type_claims: BTreeMap<String, (String, BTreeSet<String>)>,
}
impl Replay {
    fn load(db: &Connection, subject: &str, predicate: &str) -> Result<Self> {
        let mut state = Self {
            derivations: vec![],
            fields: BTreeMap::new(),
            groups: vec![],
            dependents: vec![],
            type_claims: BTreeMap::new(),
        };
        let mut slots = HashMap::<Slot, usize>::new();
        let mut ids = HashMap::<String, usize>::new();
        let mut statement = db.prepare_cached(
            "SELECT j.id, j.claim_id, c.scalar, e.source, e.scope, e.location, e.captured_at,
             e.source_updated_at, s.evidence_id, s.field, s.start, s.end, s.portable_key,
             j.extraction_rule, j.current, c.object_value, c.type_ancestors FROM claim c JOIN justification j ON j.claim_id = c.id
             JOIN justification_span s ON s.justification_id = j.id JOIN evidence e ON e.id = s.evidence_id
             WHERE c.subject = ?1 AND c.predicate = ?2")?;
        let mut rows = statement.query(params![subject, predicate])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let d = if let Some(&d) = ids.get(&id) {
                d
            } else {
                let d = state.derivations.len();
                state.derivations.push(Derivation {
                    id: id.clone(),
                    claim: r.get(1)?,
                    scalar: r.get(2)?,
                    current: r.get(14)?,
                    spans: vec![],
                    requirements: vec![],
                    groups: BTreeSet::new(),
                });
                ids.insert(id, d);
                d
            };
            if predicate == rastro_core::claims::TYPE_PREDICATE {
                let claim = &state.derivations[d].claim;
                if !state.type_claims.contains_key(claim)
                    && let Some(value) = r.get::<_, Option<String>>(15)?
                {
                    state.type_claims.insert(
                        claim.clone(),
                        (value, serde_json::from_str(&r.get::<_, String>(16)?)?),
                    );
                }
            }
            let mut span = Span {
                source: r.get(3)?,
                scope: r.get(4)?,
                location: r.get(5)?,
                event: (r.get(6)?, r.get(8)?),
                updated: r.get::<_, Option<String>>(7)?.map(parse_time).transpose()?,
                support: Support {
                    evidence_id: r.get(8)?,
                    field: r.get(9)?,
                    start: r.get::<_, i64>(10)? as u64,
                    end: r.get::<_, i64>(11)? as u64,
                    portable_key: r.get(12)?,
                    extraction_rule: r.get(13)?,
                },
                slot: 0,
                group: 0,
            };
            let key = (span.location.clone(), span.support.field.clone());
            let next = state.groups.len();
            span.group = *state.fields.entry(key).or_insert_with(|| {
                state.groups.push(Group::default());
                next
            });
            let derivation = &mut state.derivations[d];
            let slot = (
                span.source.clone(),
                span.scope.clone(),
                span.location.clone(),
                span.support.field.clone(),
                span.support.extraction_rule.clone(),
                (!derivation.scalar).then(|| derivation.claim.clone()),
            );
            let next = slots.len();
            span.slot = *slots.entry(slot).or_insert_with(|| {
                state.dependents.push(BTreeSet::new());
                next
            });
            state.dependents[span.slot].insert(d);
            state.groups[span.group].derivations.insert(d);
            derivation.groups.insert(span.group);
            derivation.spans.push(span);
        }
        for d in &mut state.derivations {
            let mut requirements = BTreeMap::new();
            for (index, span) in d.spans.iter().enumerate() {
                requirements
                    .entry((
                        span.support.portable_key.clone(),
                        span.support.field.clone(),
                        span.support.start,
                        span.support.end,
                    ))
                    .or_insert_with(Vec::new)
                    .push(index);
            }
            d.requirements = requirements.into_values().collect();
        }
        Ok(state)
    }
    fn selected(&self, seeds: Option<&BTreeSet<Field>>) -> BTreeSet<usize> {
        let Some(seeds) = seeds else {
            return (0..self.derivations.len()).collect();
        };
        let mut pending: Vec<_> = seeds
            .iter()
            .filter_map(|field| self.fields.get(field).copied())
            .collect();
        let mut groups = BTreeSet::new();
        let mut selected = BTreeSet::new();
        while let Some(group) = pending.pop() {
            if !groups.insert(group) {
                continue;
            }
            for &d in &self.groups[group].derivations {
                if selected.insert(d) {
                    pending.extend(&self.derivations[d].groups);
                }
            }
        }
        selected
    }
    fn dominance(
        &mut self,
        group: usize,
        latest: &[Option<Event>],
        blocked: &mut [usize],
        changed: &mut BTreeSet<usize>,
    ) {
        let g = &self.groups[group];
        // The best comparable timestamp from another claim dominates an older scalar.
        // Keeping the two best claims avoids comparing every candidate with every other one.
        let mut maxima = BTreeMap::<&str, Updated>::new();
        let mut minima = Vec::new();
        for &d in &g.candidates {
            let derivation = &self.derivations[d];
            let mut minimum = None;
            for span in derivation.active(latest).filter(|s| s.group == group) {
                if let Some(updated) = span.updated {
                    maxima
                        .entry(&derivation.claim)
                        .and_modify(|old| *old = (*old).max(updated))
                        .or_insert(updated);
                    minimum = Some(minimum.map_or(updated, |old: Updated| old.min(updated)));
                }
            }
            minima.push((d, minimum));
        }
        let mut best: Vec<_> = maxima.into_iter().collect();
        best.sort_unstable_by_key(|b| std::cmp::Reverse(b.1));
        let now: BTreeSet<_> = minima
            .into_iter()
            .filter_map(|(d, minimum)| {
                let derivation = &self.derivations[d];
                let other = best
                    .iter()
                    .take(2)
                    .find(|(claim, _)| *claim != derivation.claim);
                (derivation.scalar && minimum.zip(other).is_some_and(|(old, (_, new))| *new > old))
                    .then_some(d)
            })
            .collect();
        for &d in g.blocked.difference(&now) {
            blocked[d] -= 1;
            changed.insert(d);
        }
        for &d in now.difference(&g.blocked) {
            blocked[d] += 1;
            changed.insert(d);
        }
        self.groups[group].blocked = now;
    }
    fn run(&mut self, db: &Connection, selected: &BTreeSet<usize>) -> Result<()> {
        let mut delete = db.prepare_cached("DELETE FROM occurrence WHERE justification_id = ?1")?;
        let mut events = BTreeMap::<Event, Vec<(usize, usize)>>::new();
        for &d in selected {
            delete.execute([&self.derivations[d].id])?;
            self.derivations[d].current = false;
            for (s, span) in self.derivations[d].spans.iter().enumerate() {
                events.entry(span.event.clone()).or_default().push((d, s));
            }
        }
        let mut latest = vec![None; self.dependents.len()];
        let mut candidates = vec![false; self.derivations.len()];
        let mut blocked = vec![0; self.derivations.len()];
        let mut ordinals = HashMap::<(usize, String, String), u64>::new();
        let mut close = db.prepare_cached("UPDATE occurrence SET end = ?2, end_evidence_id = ?3 WHERE justification_id = ?1 AND end IS NULL")?;
        let mut open = db.prepare_cached("INSERT INTO occurrence (id, justification_id, source, scope, location, field, captured_at, source_updated_at, portable_key, evidence_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)")?;
        for (event, observations) in events {
            let portable = self.derivations[observations[0].0].spans[observations[0].1]
                .support
                .portable_key
                .clone();
            let mut changed = BTreeSet::<usize>::new();
            for (d, s) in observations {
                let slot = self.derivations[d].spans[s].slot;
                latest[slot] = Some(event.clone());
                changed.extend(&self.dependents[slot]);
            }
            let mut groups = BTreeSet::new();
            for &d in &changed {
                let live = self.derivations[d].live(&latest);
                for &group in &self.derivations[d].groups {
                    groups.insert(group);
                    if live {
                        self.groups[group].candidates.insert(d);
                    } else {
                        self.groups[group].candidates.remove(&d);
                    }
                }
                candidates[d] = live;
            }
            for group in groups {
                self.dominance(group, &latest, &mut blocked, &mut changed);
            }
            for d in changed {
                let now = candidates[d] && blocked[d] == 0;
                let derivation = &mut self.derivations[d];
                if now == derivation.current {
                    continue;
                }
                if now {
                    let span = derivation
                        .active(&latest)
                        .max_by_key(|s| &s.event)
                        .expect("live derivation has spans");
                    let ordinal = ordinals
                        .entry((d, event.0.clone(), portable.clone()))
                        .or_default();
                    let id =
                        occurrence_id_with_ordinal(&derivation.id, &event.0, &portable, *ordinal);
                    *ordinal += 1;
                    open.execute(params![
                        id,
                        derivation.id,
                        span.source,
                        span.scope,
                        span.location,
                        span.support.field,
                        event.0,
                        span.updated.map(crate::stamp),
                        portable,
                        event.1
                    ])?;
                } else {
                    close.execute(params![derivation.id, event.0, event.1])?;
                }
                derivation.current = now;
            }
        }
        let mut current =
            db.prepare_cached("UPDATE justification SET current = ?2 WHERE id = ?1")?;
        for &d in selected {
            current.execute(params![self.derivations[d].id, self.derivations[d].current])?;
        }
        Ok(())
    }
    fn compatible_types(&self, left: &str, right: &str) -> bool {
        self.type_claims
            .get(left)
            .zip(self.type_claims.get(right))
            .is_some_and(|((a, ancestors_a), (b, ancestors_b))| {
                ancestors_a.contains(b) || ancestors_b.contains(a)
            })
    }
    fn rollup(&self, db: &Connection, subject: &str, predicate: &str) -> Result<()> {
        let mut supports = BTreeMap::<&str, Vec<Support>>::new();
        for d in &self.derivations {
            let entry = supports.entry(&d.claim).or_default();
            if d.current {
                entry.extend(d.spans.iter().map(|s| s.support.clone()));
            }
        }
        let mut edges = BTreeSet::new();
        let is_type = predicate == rastro_core::claims::TYPE_PREDICATE;
        for group in &self.groups {
            let mut current = BTreeSet::new();
            let mut old = BTreeSet::new();
            for &d in &group.derivations {
                let d = &self.derivations[d];
                if !d.scalar {
                    continue;
                }
                if d.current {
                    current.insert(d.claim.as_str());
                }
                if supports[d.claim.as_str()].is_empty() {
                    old.insert(d.claim.as_str());
                }
            }
            for &from in &current {
                for &to in &old {
                    if from != to {
                        edges.insert((from, to, "supersedes"));
                    }
                }
                // A type conflicts with every unrelated type live anywhere in the component.
                if is_type {
                    continue;
                }
                for &to in &current {
                    if from != to && !self.compatible_types(from, to) {
                        edges.insert((from, to, "conflicts_with"));
                    }
                }
            }
        }
        if is_type {
            let live: BTreeSet<&str> = self
                .derivations
                .iter()
                .filter(|d| d.scalar && d.current)
                .map(|d| d.claim.as_str())
                .collect();
            for &from in &live {
                for &to in &live {
                    if from != to && !self.compatible_types(from, to) {
                        edges.insert((from, to, "conflicts_with"));
                    }
                }
            }
        }
        db.execute("DELETE FROM claim_link WHERE from_claim IN (SELECT id FROM claim WHERE subject = ?1 AND predicate = ?2) OR to_claim IN (SELECT id FROM claim WHERE subject = ?1 AND predicate = ?2)", params![subject, predicate])?;
        let mut visible = BTreeMap::<&str, Vec<(String, String)>>::new();
        let mut insert = db.prepare_cached("INSERT INTO claim_link VALUES (?1, ?2, ?3)")?;
        for (from, to, kind) in edges {
            insert.execute(params![from, to, kind])?;
            visible
                .entry(from)
                .or_default()
                .push((kind.into(), to.into()));
            if kind == "supersedes" {
                visible
                    .entry(to)
                    .or_default()
                    .push(("superseded_by".into(), from.into()));
            }
        }
        let mut update = db.prepare_cached(
            "UPDATE claim SET status = ?2, revision = ?3, known_since = ?4 WHERE id = ?1",
        )?;
        for (id, support) in supports {
            let status = if support.is_empty() {
                "superseded"
            } else {
                "observed"
            };
            let links = visible.get(id).map_or(&[][..], Vec::as_slice);
            let known_since = intervals(db, id)?
                .last()
                .map(|i| crate::stamp(i.start))
                .unwrap_or_default();
            update.execute(params![
                id,
                status,
                revision_with_links(status, &support, links),
                known_since
            ])?;
        }
        Ok(())
    }
}

/// Complete replay oracle, also used by exact scoped projections.
pub(crate) fn refresh(db: &Connection, affected: &BTreeSet<Component>) -> Result<()> {
    for (subject, predicate) in affected {
        recompute(db, subject, predicate, None)?;
    }
    Ok(())
}
pub(crate) fn refresh_groups(db: &Connection, affected: &BTreeSet<AffectedGroup>) -> Result<()> {
    let mut components = BTreeMap::<Component, BTreeSet<Field>>::new();
    for (subject, predicate, location, field) in affected {
        components
            .entry((subject.clone(), predicate.clone()))
            .or_default()
            .insert((location.clone(), field.clone()));
    }
    for ((subject, predicate), fields) in components {
        recompute(db, &subject, &predicate, Some(&fields))?;
    }
    Ok(())
}
fn recompute(
    db: &Connection,
    subject: &str,
    predicate: &str,
    fields: Option<&BTreeSet<Field>>,
) -> Result<()> {
    let mut replay = Replay::load(db, subject, predicate)?;
    let selected = replay.selected(fields);
    replay.run(db, &selected)?;
    replay.rollup(db, subject, predicate)
}
pub(crate) fn intervals(db: &Connection, id: &str) -> Result<Vec<KnownInterval>> {
    // An owner intent holds its claim over stretches: from its issue (or, for a name restored by
    // undoing a newer one, that undo) until an undo or a newer name, with no end while it holds.
    let rows = db
        .prepare(
            "SELECT captured_at, evidence_id, end, end_evidence_id FROM (
        SELECT o.captured_at, o.evidence_id, o.end, o.end_evidence_id, o.id FROM occurrence o
        JOIN justification j ON j.id = o.justification_id WHERE j.claim_id = ?1
        UNION ALL SELECT coalesce(ij.started_at, i.issued_at), 0, ij.ended_at, CASE WHEN ij.ended_at IS NULL THEN NULL ELSE 0 END, ij.id
        FROM intent_justification ij JOIN intent i ON i.id = ij.intent_id WHERE ij.claim_id = ?1)
        ORDER BY captured_at, evidence_id, id",
        )?
        .query_map([id], |r| {
            Ok((
                (r.get::<_, String>(0)?, r.get::<_, i64>(1)?),
                r.get::<_, Option<String>>(2)?
                    .zip(r.get::<_, Option<i64>>(3)?),
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    // Union on event keys, preserving a gap even if its timestamps are equal.
    let mut intervals: Vec<(Event, Option<Event>)> = Vec::new();
    for (start, end) in rows {
        if let Some((_, last_end)) = intervals.last_mut()
            && last_end.as_ref().is_none_or(|e| e >= &start)
        {
            if end.is_none()
                || last_end
                    .as_ref()
                    .is_some_and(|old| end.as_ref().is_some_and(|new| new > old))
            {
                *last_end = end;
            }
            continue;
        }
        intervals.push((start, end));
    }
    intervals
        .into_iter()
        .map(|(start, end)| {
            Ok(KnownInterval {
                start: parse_time(start.0)?,
                end: end.map(|e| parse_time(e.0)).transpose()?,
            })
        })
        .collect()
}
pub(crate) fn links(db: &Connection, id: &str, kind: &str, inverse: bool) -> Result<Vec<String>> {
    let sql = if inverse {
        "SELECT from_claim FROM claim_link WHERE to_claim = ?1 AND kind = ?2 ORDER BY from_claim"
    } else {
        "SELECT to_claim FROM claim_link WHERE from_claim = ?1 AND kind = ?2 ORDER BY to_claim"
    };
    Ok(db
        .prepare(sql)?
        .query_map(params![id, kind], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

const TABLES: &[&str] = &[
    "entity",
    "claim",
    "justification",
    "justification_span",
    "occurrence",
    "claim_link",
    "intent_justification",
    "extraction_checkpoint",
];
pub(crate) fn staging(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_temp_master WHERE name = 'claim')",
        [],
        |r| r.get(0),
    )?)
}
pub(crate) fn publish(db: &Connection) -> Result<()> {
    let extractors: Vec<(String, String)> = db.prepare("SELECT DISTINCT extractor, extractor_version FROM extraction_checkpoint ORDER BY extractor, extractor_version")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let vocabularies: Vec<(String, String)> = db.prepare("SELECT DISTINCT extractor, vocabulary_version FROM extraction_checkpoint ORDER BY extractor, vocabulary_version")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let high_water: Vec<(String, String, String, String)> = db.prepare("SELECT source, scope, last_captured_at, last_portable_key FROM extraction_checkpoint ORDER BY source, scope, extractor")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
    db.execute("INSERT INTO main.projection_publication (extractor_versions, vocabulary_versions, evidence_high_water) VALUES (?1, ?2, ?3)",
        params![serde_json::to_string(&extractors)?, serde_json::to_string(&vocabularies)?, serde_json::to_string(&high_water)?])?;
    for table in TABLES {
        db.execute_batch(&format!("ANALYZE main.{table}"))?;
    }
    Ok(())
}
/// Copies one pair's stored output into the staged generation: its justifications and spans,
/// the claims and entities they reach, and the pair's checkpoint. Statuses, occurrences and
/// links are not copied; every component the pair touches is recomputed from the staged
/// justifications, which may now include other pairs'. Returns the justifications carried.
pub(crate) fn carry_over(
    db: &Connection,
    source: &str,
    scope: &str,
    extractor: &str,
) -> Result<usize> {
    anyhow::ensure!(staging(db)?, "no claims rebuild is staged");
    // The pair's justifications, found once; everything else is driven from them.
    db.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS carry_ids (id TEXT PRIMARY KEY); DELETE FROM temp.carry_ids",
    )?;
    let result = (|| -> Result<usize> {
        let with = params![source, scope, extractor];
        db.execute(
            "INSERT INTO temp.carry_ids SELECT j.id FROM main.justification j WHERE j.extractor = ?3 AND EXISTS (
                SELECT 1 FROM main.justification_span s JOIN main.evidence e ON e.id = s.evidence_id
                WHERE s.justification_id = j.id AND e.source = ?1 AND e.scope = ?2)",
            with,
        )?;
        const CLAIMS: &str =
            "SELECT j.claim_id FROM main.justification j JOIN temp.carry_ids c ON c.id = j.id";
        db.execute(
            &format!("INSERT OR IGNORE INTO temp.entity (id, identifier)
                SELECT id, identifier FROM main.entity WHERE id IN
                (SELECT subject FROM main.claim WHERE id IN ({CLAIMS})
                 UNION SELECT object_entity FROM main.claim WHERE id IN ({CLAIMS}) AND object_entity IS NOT NULL)"),
            [],
        )?;
        db.execute(
            &format!(
                "INSERT OR IGNORE INTO temp.claim SELECT * FROM main.claim WHERE id IN ({CLAIMS})"
            ),
            [],
        )?;
        let carried = db.execute(
            "INSERT OR IGNORE INTO temp.justification SELECT j.* FROM main.justification j JOIN temp.carry_ids c ON c.id = j.id",
            [],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO temp.justification_span SELECT s.* FROM main.justification_span s
             JOIN temp.carry_ids c ON c.id = s.justification_id",
            [],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO temp.extraction_checkpoint SELECT * FROM main.extraction_checkpoint
             WHERE source = ?1 AND scope = ?2 AND extractor = ?3",
            with,
        )?;
        let touched: BTreeSet<Component> = db
            .prepare(&format!(
                "SELECT DISTINCT subject, predicate FROM temp.claim WHERE id IN ({CLAIMS})"
            ))?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        refresh(db, &touched)?;
        Ok(carried)
    })();
    db.execute_batch("DROP TABLE IF EXISTS temp.carry_ids")?;
    result
}
pub(crate) fn begin_rebuild(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!("projection_temp.sql"))?;
    Ok(())
}
pub(crate) fn finish_rebuild(db: &mut Connection, publish_projection: bool) -> Result<usize> {
    // Immediate: the intents replayed below are the ones in force when the swap commits.
    let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // Intents replay after the evidence and every carried-over pair, before publication.
    if publish_projection {
        crate::identity::apply(&transaction)?;
    }
    let count: i64 = transaction.query_row("SELECT count(*) FROM temp.claim", [], |r| r.get(0))?;
    if publish_projection {
        for table in TABLES.iter().rev() {
            transaction.execute(&format!("DELETE FROM main.{table}"), [])?;
        }
        for table in TABLES {
            transaction.execute(
                &format!("INSERT INTO main.{table} SELECT * FROM temp.{table}"),
                [],
            )?;
        }
    }
    for table in TABLES.iter().rev() {
        transaction.execute_batch(&format!("DROP TABLE temp.{table}"))?;
    }
    if publish_projection {
        publish(&transaction)?;
    }
    transaction.commit()?;
    Ok(count as usize)
}
