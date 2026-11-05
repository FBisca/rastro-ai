//! Queries over the published, unscoped projection. Never mutates lifecycle state.
use super::*;
use rastro_core::knowledge::{
    AboutOptions, Citation, ClaimSummary, Explanation, JustificationView, WhyOptions,
};

impl View<'_> {
    fn aliases(&self, entity: &str) -> Result<Vec<String>> {
        Ok(self.0.prepare("SELECT DISTINCT object_value FROM claim WHERE subject = ?1 AND predicate = 'rastro:alias' AND status = 'observed' ORDER BY object_value")?
            .query_map([entity], |row| row.get(0))?.collect::<Result<_, _>>()?)
    }
    fn entity(&self, id: &str) -> Result<Entity> {
        let (identifier, entity_type) = self.0.query_row("SELECT identifier, coalesce((SELECT object_value FROM claim WHERE subject = e.id AND predicate = 'rastro:type' AND status = 'observed' ORDER BY id LIMIT 1), '') FROM entity e WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(Entity::new(identifier, entity_type))
    }
    fn summary(&self, id: &str) -> Result<Option<ClaimSummary>> {
        let row = self.0.query_row("SELECT subject, predicate, object_entity, object_value, object_datatype, status, revision, (SELECT count(*) FROM justification WHERE claim_id = c.id AND current = 1) FROM claim c WHERE id = ?1", [id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, i64>(7)?))).optional()?;
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
    pub(super) fn about_claims(
        &self,
        entity: &str,
        options: &AboutOptions,
    ) -> Result<Vec<ClaimSummary>> {
        let exact: Option<String> = self.0.query_row(
            "SELECT id FROM entity WHERE id = ?1 OR identifier = ?1 ORDER BY (id = ?1) DESC LIMIT 1", [entity], |row| row.get(0)).optional()?;
        let resolved = match exact {
            Some(id) => Some(id),
            None => {
                let candidates = self.0.prepare("SELECT DISTINCT e.id, e.identifier FROM claim c JOIN entity e ON e.id = c.subject WHERE c.predicate = 'rastro:alias' AND c.status = 'observed' AND c.object_value = ?1 ORDER BY e.id")?
                    .query_map([entity], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                if candidates.len() > 1 {
                    bail!(
                        "ambiguous alias {entity}; candidates: {}",
                        candidates
                            .iter()
                            .map(|(id, identifier)| format!("{id} ({identifier})"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                candidates.into_iter().next().map(|(id, _)| id)
            }
        };
        let Some(entity) = resolved else {
            return Ok(vec![]);
        };
        let since = options.since.map(crate::stamp);
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
        ids.into_iter()
            .map(|id| {
                self.summary(&id)?
                    .ok_or_else(|| anyhow::anyhow!("claim missing in snapshot"))
            })
            .collect()
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
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
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
