//! Queries over the published, unscoped projection. Never mutates lifecycle state.
use super::*;
use anyhow::ensure;
use rastro_core::knowledge::{
    AboutClaim, AboutClaims, AboutOptions, Citation, ClaimSummary, Explanation, JustificationView,
    LinkReason, LinkReasonKind, LinkedAccount, PersonView, WhyOptions,
};
use std::collections::BTreeSet;

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
const CURRENT: &str = "('observed', 'confirmed')";
impl View<'_> {
    fn aliases(&self, entity: &str) -> Result<Vec<String>> {
        Ok(self.0.prepare(&format!("SELECT DISTINCT object_value FROM claim WHERE subject = ?1 AND predicate = 'rastro:alias' AND status IN {CURRENT} ORDER BY object_value"))?
            .query_map([entity], |row| row.get(0))?.collect::<Result<_, _>>()?)
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
        let row = self.0.query_row("SELECT subject, predicate, object_entity, object_value, object_datatype, status, revision, (SELECT count(*) FROM justification WHERE claim_id = c.id AND current = 1) + (SELECT count(*) FROM intent_justification WHERE claim_id = c.id AND ended_at IS NULL) FROM claim c WHERE id = ?1", [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, i64>(7)?))).optional()?;
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
            },
            subject_aliases,
            object_aliases,
            support_count: count as usize,
        }))
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
            "SELECT subject, object_entity FROM claim WHERE predicate = ?2 AND status = 'observed' \
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
    pub(super) fn about_view(
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
                 AND (?2 OR status = 'observed') AND (?3 IS NULL OR known_since >= ?3) \
                 ORDER BY known_since DESC, id LIMIT ?4",
            )?;
            let ids = statement
                .query_map(
                    params![entity, options.history, since, (options.limit + 1) as i64],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(AboutClaims {
                resolved: true,
                claims: summaries(ids, None)?,
                person: None,
            });
        }
        let class = self.merge_class(&entity)?;
        let accounts: BTreeSet<String> = self
            .0
            .prepare(
                "SELECT DISTINCT subject FROM claim WHERE predicate = ?2 AND status = 'observed' \
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
        let view: BTreeSet<&String> = class.iter().chain(&accounts).collect();
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
                 AND (?2 OR c.status = 'observed') AND (?3 IS NULL OR c.known_since >= ?3) \
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
            // Today a link's only reason is an owner intent that still holds.
            let reasons = self
                .0
                .prepare(
                    "SELECT c.id, ij.id, ij.intent_id FROM claim c JOIN intent_justification ij ON ij.claim_id = c.id \
                     WHERE c.subject = ?1 AND c.predicate = ?3 AND c.status = 'observed' AND ij.ended_at IS NULL \
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
        })
    }

    pub(super) fn explanation(
        &self,
        claim: &str,
        options: &WhyOptions,
    ) -> Result<Option<Explanation>> {
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
                justification_kind: rastro_core::identity::OWNER_INTENT.into(),
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
        supports.sort_by(|a, b| {
            (!a.current, &a.justification_id).cmp(&(!b.current, &b.justification_id))
        });
        let truncated = supports.len() > options.limit;
        supports.truncate(options.limit);
        for support in &mut supports {
            support.spans = self.citations(&support.justification_id, options.full)?;
        }
        Ok(Some(Explanation {
            claim: Box::new(claim),
            supports,
            truncated,
            coverage: vec![],
            extraction: vec![],
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
