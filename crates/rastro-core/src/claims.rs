//! Claims: what an extractor derives from evidence, checked by the core before anything is
//! stored (ADR 0008), with content-addressed ids (ADR 0009).
use crate::{Evidence, Scope, TrustClass};
use chrono::{DateTime, Utc};
use rastro_plugin_sdk::{ClaimObject, ClaimSubmit, Manifest, Vocabulary};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};

/// The longest identifier, predicate or literal value a claim may carry, in UTF-8 bytes.
pub const MAX_TERM_BYTES: usize = 4096;

/// An entity: one exact canonical identifier, with its type.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Entity {
    pub id: String,
    pub identifier: String,
    #[serde(rename = "type")]
    pub entity_type: String,
}
impl Entity {
    pub fn new(identifier: String, entity_type: String) -> Self {
        Self {
            id: entity_id(&identifier),
            identifier,
            entity_type,
        }
    }
}
/// What a claim says about its subject.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Object {
    Entity {
        id: String,
        identifier: String,
        #[serde(rename = "type")]
        entity_type: String,
    },
    Literal {
        value: String,
        datatype: String,
    },
}
/// A byte range of one observation's field that supports a claim.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Support {
    pub evidence_id: i64,
    pub field: String,
    pub start: u64,
    pub end: u64,
    pub extraction_rule: String,
}
/// A claim the core accepted as `observed`, ready to store.
#[derive(Clone, Debug)]
pub struct ObservedClaim {
    pub scalar: bool,
    pub id: String,
    pub subject: Entity,
    pub predicate: String,
    pub object: Object,
    pub supports: BTreeSet<Support>,
}
/// How far an extractor version has read a (source, scope): the last observation processed,
/// in replay order (`captured_at`, evidence id).
pub struct Checkpoint {
    pub extractor: String,
    pub extractor_version: String,
    pub source: String,
    pub scope: Scope,
    pub last: (DateTime<Utc>, i64),
}

/// Canonical encoding, version 1: a domain tag, then each part as one tag byte, its length in
/// bytes as a big-endian u64, and its UTF-8 bytes. The id is a prefix plus the first 16 bytes
/// of the SHA-256 of that, in hex. Changing any of this is a new version (new domain tags).
fn digest(prefix: &str, domain: &str, parts: &[(u8, &str)]) -> String {
    let mut hash = Sha256::new();
    for (tag, text) in std::iter::once((b'D', domain)).chain(parts.iter().copied()) {
        hash.update([tag]);
        hash.update((text.len() as u64).to_be_bytes());
        hash.update(text.as_bytes());
    }
    let bytes = hash.finalize();
    let hex: String = bytes[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}{hex}")
}
/// Entity id: the hash of its canonical identifier.
pub fn entity_id(identifier: &str) -> String {
    digest("e_", "rastro/entity/v1", &[(b'I', identifier)])
}
/// Claim id: the hash of subject id, predicate and typed object (an entity id, or a datatype
/// and a value), so the same statement has the same id whichever run or rebuild makes it.
pub fn claim_id(subject: &str, predicate: &str, object: &Object) -> String {
    let mut parts = vec![(b'S', subject), (b'P', predicate)];
    match object {
        Object::Entity { id, .. } => parts.push((b'E', id)),
        Object::Literal { value, datatype } => parts.extend([(b'T', &**datatype), (b'L', value)]),
    }
    digest("c_", "rastro/claim/v1", &parts)
}
/// Revision: the hash of a claim's visible state, its status and the set of its current
/// supports, so a second support changes it.
pub fn revision(status: &str, supports: &[Support]) -> String {
    revision_with_links(status, supports, &[])
}

/// Extends the v1 encoding with sorted (kind, target) link parts.
pub fn revision_with_links(
    status: &str,
    supports: &[Support],
    links: &[(String, String)],
) -> String {
    let mut supports = supports.to_vec();
    supports.sort();
    let numbers: Vec<_> = supports
        .iter()
        .map(|s| {
            [
                s.evidence_id.to_string(),
                s.start.to_string(),
                s.end.to_string(),
            ]
        })
        .collect();
    let mut parts = vec![(b'A', status)];
    for (support, [evidence, start, end]) in supports.iter().zip(&numbers) {
        parts.extend([
            (b'V', &**evidence),
            (b'F', &*support.field),
            (b'B', &**start),
            (b'N', &**end),
            (b'R', &*support.extraction_rule),
        ]);
    }
    let mut links = links.to_vec();
    links.sort();
    for (kind, target) in &links {
        parts.extend([(b'K', kind.as_str()), (b'C', target.as_str())]);
    }
    digest("r_", "rastro/revision/v1", &parts)
}

/// Checks a submission against what this run delivered, the extractor's declared rules and
/// its vocabulary. `Err` carries the reason it is rejected.
pub fn validate(
    submit: ClaimSubmit,
    delivered: &HashMap<i64, Evidence>,
    manifest: &Manifest,
    vocabulary: &Vocabulary,
) -> Result<ObservedClaim, String> {
    let terms = [&submit.subject.identifier, &submit.predicate];
    let object_term = match &submit.object {
        ClaimObject::Entity { identifier, .. } => identifier,
        ClaimObject::Literal { value, .. } => value,
    };
    if terms
        .into_iter()
        .chain([object_term])
        .any(|t| t.len() > MAX_TERM_BYTES)
    {
        return Err(format!(
            "a claim term is longer than {MAX_TERM_BYTES} bytes"
        ));
    }
    let rule = manifest
        .extraction_rules
        .iter()
        .find(|r| r.id == submit.extraction_rule)
        .ok_or("extraction rule not declared")?;
    if !rule.deterministic {
        return Err("extraction rule not declared deterministic".into());
    }
    let predicate = vocabulary
        .predicates
        .iter()
        .find(|p| p.id == submit.predicate)
        .ok_or("predicate not in the vocabulary")?;
    let typed = |t: &String| vocabulary.types.contains(t);
    let subject = submit.subject;
    if subject.identifier.is_empty()
        || !typed(&subject.entity_type)
        || !predicate.domain.contains(&subject.entity_type)
    {
        return Err("subject outside the predicate's domain".into());
    }
    let object = match submit.object {
        ClaimObject::Entity {
            identifier,
            entity_type,
        } if !identifier.is_empty()
            && typed(&entity_type)
            && predicate.range.contains(&entity_type) =>
        {
            let Entity { id, .. } = Entity::new(identifier.clone(), entity_type.clone());
            Object::Entity {
                id,
                identifier,
                entity_type,
            }
        }
        ClaimObject::Literal { value, datatype }
            if !typed(&datatype) && predicate.range.contains(&datatype) =>
        {
            Object::Literal { value, datatype }
        }
        _ => return Err("object outside the predicate's range".into()),
    };
    if submit.supports.is_empty() {
        return Err("a claim needs at least one support".into());
    }
    let mut supports = BTreeSet::new();
    for support in submit.supports {
        let evidence = delivered
            .get(&support.evidence_id)
            .ok_or("support names evidence not delivered to this run")?;
        if evidence.trust_class != TrustClass::Structured {
            return Err("support is not structured evidence".into());
        }
        let inside = support.field == evidence.field
            && evidence.start <= support.start
            && support.start < support.end
            && support.end <= evidence.end;
        let at = |offset: u64| {
            evidence
                .quote
                .is_char_boundary((offset - evidence.start) as usize)
        };
        if !inside || !at(support.start) || !at(support.end) {
            return Err("support span lies outside its evidence".into());
        }
        supports.insert(Support {
            evidence_id: support.evidence_id,
            field: support.field,
            start: support.start,
            end: support.end,
            extraction_rule: rule.id.clone(),
        });
    }
    let subject = Entity::new(subject.identifier, subject.entity_type);
    Ok(ObservedClaim {
        scalar: predicate.cardinality == rastro_plugin_sdk::Cardinality::One,
        id: claim_id(&subject.id, &submit.predicate, &object),
        subject,
        predicate: submit.predicate,
        object,
        supports,
    })
}

/// A claim as `why` shows it.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct ClaimView {
    pub id: String,
    pub subject: Entity,
    pub predicate: String,
    pub object: Object,
    #[schemars(extend("enum" = ["observed", "superseded"]))]
    pub status: String,
    pub revision: String,
    pub known_intervals: Vec<KnownInterval>,
    pub valid_time: ValidTime,
    pub conflicts_with: Vec<String>,
    pub supersedes: Vec<String>,
    pub superseded_by: Vec<String>,
}
/// When evidence established and replaced current support for a claim.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct KnownInterval {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}
/// Item update times do not establish when individual fields changed in the world.
#[derive(Clone, Debug, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidTime {
    Unknown,
}
/// One support of a claim, with the evidence it cites.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct SupportView {
    pub current: bool,
    pub evidence_id: i64,
    pub source: String,
    pub scope: Scope,
    pub location: String,
    pub field: String,
    pub start: u64,
    pub end: u64,
    /// The supporting bytes of the observation.
    pub quote: String,
    pub trust_class: TrustClass,
    pub captured_at: DateTime<Utc>,
    pub source_updated_at: Option<DateTime<Utc>>,
    pub extraction_rule: String,
    pub extractor: String,
    pub extractor_version: String,
}
/// A claim and every support it has.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct Why {
    pub claim: ClaimView,
    pub supports: Vec<SupportView>,
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// The canonical encoding is fixed: these ids (checked against an independent SHA-256 of
    /// the encoding) must never change within version 1.
    #[test]
    fn S7_ids_are_stable_across_builds() {
        let pr = entity_id("https://github.com/o/p/pull/1");
        assert_eq!(pr, "e_e57fd39600ca2d8bb660fe3b66cc4aed");
        let open = Object::Literal {
            value: "open".into(),
            datatype: "string".into(),
        };
        assert_eq!(
            claim_id(&pr, "code:state", &open),
            "c_396068c0fa399a0ab85fba670ab6abf3"
        );
    }
}
