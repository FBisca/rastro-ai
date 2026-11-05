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
pub const TYPE_PREDICATE: &str = "rastro:type";
pub const ALIAS_PREDICATE: &str = "rastro:alias";
/// The datatype of a `rastro:type` claim's literal: a type of the extractor's vocabulary, equal
/// to the subject's type. An entity's type is this claim, not a column, and is never updated in
/// place. The core owns the `rastro:` prefix; no vocabulary may define a name under it. A
/// `rastro:alias` claim's datatype is an alias kind the vocabulary declares, and its value is
/// exactly the text one of its supports quotes.
pub const TYPE_DATATYPE: &str = "rastro:type";

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
    pub portable_key: String,
    pub evidence_id: i64,
    pub field: String,
    pub start: u64,
    pub end: u64,
    pub extraction_rule: String,
}
/// A claim the core accepted as `observed`, ready to store.
#[derive(Clone, Debug)]
pub struct ObservedClaim {
    /// Ancestor type names supplied by vocabulary validation, never part of an id.
    pub type_ancestors: Vec<String>,
    pub scalar: bool,
    pub id: String,
    pub subject: Entity,
    pub predicate: String,
    pub object: Object,
    pub supports: BTreeSet<Support>,
}
/// How far an extractor version has read a pair. `last.0` and `last_portable_key` name
/// the portable identity of the boundary; `last.1` is its local append high-water.
/// Ordering is (`captured_at`, local append id); ids never enter a content hash.
pub struct Checkpoint {
    pub extractor: String,
    pub extractor_version: String,
    pub source: String,
    pub scope: Scope,
    pub last: (DateTime<Utc>, i64),
    pub vocabulary_version: String,
    pub last_portable_key: String,
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

/// Revision v2 hashes a set of portable spans, never local ids or interpretation versions.
pub fn revision_with_links(
    status: &str,
    supports: &[Support],
    links: &[(String, String)],
) -> String {
    let keys: BTreeSet<_> = supports.iter().map(|s| s.portable_key.as_str()).collect();
    let mut parts = vec![(b'A', status), (b'T', "unknown")];
    parts.extend(keys.into_iter().map(|key| (b'V', key)));
    let links: BTreeSet<_> = links.iter().collect();
    for (kind, target) in links {
        parts.extend([(b'K', kind.as_str()), (b'C', target.as_str())]);
    }
    digest("r_", "rastro/revision/v2", &parts)
}

/// Portable identity of an observation; capture order never depends on a local row id.
pub fn portable_evidence_key(evidence: &Evidence) -> String {
    portable_span_key(evidence, evidence.start, evidence.end)
}
pub fn portable_span_key(e: &Evidence, start: u64, end: u64) -> String {
    let quote = &e.quote[(start - e.start) as usize..(end - e.start) as usize];
    digest(
        "v_",
        "rastro/evidence/v1",
        &[
            (b'S', &e.source),
            (b'C', &e.scope.0),
            (b'L', &e.location),
            (b'F', &e.field),
            (b'B', &start.to_string()),
            (b'N', &end.to_string()),
            (b'Q', quote),
            (
                b'T',
                &e.captured_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            ),
        ],
    )
}
/// One submission is one derivation. Rule and sorted span keys distinguish alternatives.
pub fn derivation_key(supports: &BTreeSet<Support>) -> String {
    let keys: BTreeSet<_> = supports
        .iter()
        .map(|s| {
            (
                &s.extraction_rule,
                &s.portable_key,
                &s.field,
                s.start.to_string(),
                s.end.to_string(),
            )
        })
        .collect();
    let parts: Vec<_> = keys
        .iter()
        .flat_map(|(rule, key, field, start, end)| {
            [
                (b'R', rule.as_str()),
                (b'V', key.as_str()),
                (b'F', field.as_str()),
                (b'B', start.as_str()),
                (b'N', end.as_str()),
            ]
        })
        .collect();
    digest("d_", "rastro/derivation/v1", &parts)
}
pub fn justification_id(claim: &str, extractor: &str, derivation: &str) -> String {
    digest(
        "j_",
        "rastro/justification/v1",
        &[(b'C', claim), (b'E', extractor), (b'D', derivation)],
    )
}
pub fn occurrence_id(justification: &str, captured: &str, portable_key: &str) -> String {
    digest(
        "o_",
        "rastro/occurrence/v1",
        &[
            (b'J', justification),
            (b'T', captured),
            (b'V', portable_key),
        ],
    )
}
/// Repeated openings on the same portable event keep separate, portable identities.
/// The first occurrence retains the original encoding.
pub fn occurrence_id_with_ordinal(
    justification: &str,
    captured: &str,
    portable_key: &str,
    ordinal: u64,
) -> String {
    if ordinal == 0 {
        return occurrence_id(justification, captured, portable_key);
    }
    digest(
        "o_",
        "rastro/occurrence/v1",
        &[
            (b'J', justification),
            (b'T', captured),
            (b'V', portable_key),
            (b'N', &ordinal.to_string()),
        ],
    )
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
    let typed = |t: &String| vocabulary.has_type(t);
    let subject = submit.subject;
    crate::entity_form::check(&subject.identifier)?;
    if !typed(&subject.entity_type) {
        return Err("subject outside the predicate's domain".into());
    }
    // The text an alias claim's supports must quote.
    let mut alias = None;
    // Every `rastro:type` claim is scalar: a newer type read from the same field replaces the
    // older, and two live types conflict unless the vocabulary relates them by `supertype`.
    // The projection can't see the vocabulary, so a type claim carries its supertypes.
    let scalar;
    let mut type_ancestors = Vec::new();
    let object = match (submit.predicate.as_str(), submit.object) {
        (TYPE_PREDICATE, ClaimObject::Literal { value, datatype })
            if datatype == TYPE_DATATYPE && value == subject.entity_type =>
        {
            scalar = true;
            type_ancestors = vocabulary.ancestors(&value);
            Object::Literal { value, datatype }
        }
        (TYPE_PREDICATE, _) => return Err("a type claim names its subject's own type".into()),
        (ALIAS_PREDICATE, ClaimObject::Literal { value, datatype }) => {
            let kind = vocabulary
                .aliases
                .iter()
                .find(|k| k.kind == datatype)
                .ok_or("alias kind not in the vocabulary")?;
            if !kind.domain.contains(&subject.entity_type) {
                return Err("subject outside the alias kind's domain".into());
            }
            if !crate::entity_form::is_alias_value(&value) {
                return Err(
                    "an alias value must be non-empty, with no whitespace at either end".into(),
                );
            }
            // A new observation of the field an alias is read from ends the previous alias
            // there, so a rename's two logins show when each held.
            scalar = true;
            alias = Some(value.clone());
            Object::Literal { value, datatype }
        }
        (ALIAS_PREDICATE, _) => return Err("an alias is a literal".into()),
        (id, object) => {
            let predicate = vocabulary
                .predicates
                .iter()
                .find(|p| p.id == id)
                .ok_or("predicate not in the vocabulary")?;
            if !predicate.domain.contains(&subject.entity_type) {
                return Err("subject outside the predicate's domain".into());
            }
            scalar = predicate.cardinality == rastro_plugin_sdk::Cardinality::One;
            match object {
                ClaimObject::Entity {
                    identifier,
                    entity_type,
                } if crate::entity_form::check(&identifier).is_ok()
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
            }
        }
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
            portable_key: portable_evidence_key(evidence),
            evidence_id: support.evidence_id,
            field: support.field,
            start: support.start,
            end: support.end,
            extraction_rule: rule.id.clone(),
        });
    }
    if let Some(alias) = alias {
        let quoted = |s: &Support| {
            let e = &delivered[&s.evidence_id];
            e.quote
                .get((s.start - e.start) as usize..(s.end - e.start) as usize)
                == Some(alias.as_str())
        };
        if !supports.iter().any(quoted) {
            return Err("an alias is exactly the text one of its supports quotes".into());
        }
    }
    let subject = Entity::new(subject.identifier, subject.entity_type);
    Ok(ObservedClaim {
        type_ancestors,
        scalar,
        id: claim_id(&subject.id, &submit.predicate, &object),
        subject,
        predicate: submit.predicate,
        object,
        supports,
    })
}

/// Type grounding: a submission's claims are validated together, and an endpoint's type comes
/// from a `rastro:type` claim in the same submission, never from stored state. Returns the
/// claims that pass and how many were rejected: those with a subject or entity object no
/// `rastro:type` claim here types as the claim says. Type claims ground themselves.
pub fn ground(claims: Vec<ObservedClaim>) -> (Vec<ObservedClaim>, u64) {
    let typed: BTreeSet<(String, String)> = claims
        .iter()
        .filter_map(|c| match (&*c.predicate, &c.object) {
            (TYPE_PREDICATE, Object::Literal { value, .. }) => {
                Some((c.subject.identifier.clone(), value.clone()))
            }
            _ => None,
        })
        .collect();
    let grounded = |e: &Entity| typed.contains(&(e.identifier.clone(), e.entity_type.clone()));
    let (kept, rejected): (Vec<_>, Vec<_>) = claims.into_iter().partition(|c| {
        c.predicate == TYPE_PREDICATE
            || grounded(&c.subject)
                && match &c.object {
                    Object::Entity {
                        identifier,
                        entity_type,
                        ..
                    } => typed.contains(&(identifier.clone(), entity_type.clone())),
                    Object::Literal { .. } => true,
                }
    });
    (kept, rejected.len() as u64)
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
#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// claims/R10: revision ignores local ids and rules
    #[test]
    fn revision_ignores_local_ids_and_rules() {
        let first = Support {
            portable_key: "portable-span".into(),
            evidence_id: 1,
            field: "/state".into(),
            start: 0,
            end: 4,
            extraction_rule: "a".into(),
        };
        let mut replay = first.clone();
        replay.evidence_id = 42;
        replay.extraction_rule = "b".into();
        assert_eq!(
            revision("observed", &[first]),
            revision("observed", &[replay])
        );
    }

    /// claims/R8: ids are stable across builds. The canonical encoding is fixed: these ids (checked against an independent SHA-256 of
    /// the encoding) must never change within version 1.
    #[test]
    fn ids_are_stable_across_builds() {
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

    const WORDS: &str = "definition = \"d\"\nnot_to_confuse_with = \"n\"\nexamples = [\"e\"]\n\
        counter_examples = [\"c\"]\nevidence_requirement = \"r\"\n";
    fn vocabulary() -> Vocabulary {
        Vocabulary::parse(&format!(
            "namespace = \"x\"\nversion = \"1\"\npredicates = []\n\
             [[types]]\nid = \"x:Account\"\n{WORDS}[[types]]\nid = \"x:Item\"\n{WORDS}\
             [[aliases]]\nkind = \"login\"\ndomain = [\"x:Account\"]\n{WORDS}"
        ))
        .unwrap()
    }
    fn manifest() -> Manifest {
        Manifest::parse(
            "id = \"x\"\nname = \"x\"\nversion = \"1\"\nauthor = \"x\"\nkinds = []\n\
             protocol = 1\ncommand = \"x\"\ntriggers = []\ndependencies = [\"x\"]\n\
             [[extraction_rules]]\nid = \"r\"\ndeterministic = true\n\
             [permissions]\nsubmit = []\nscopes = []\nactions = []\n",
        )
        .unwrap()
    }
    /// One delivered observation: evidence 1, `/user/login` quoting `octocat`.
    fn delivered() -> HashMap<i64, Evidence> {
        let evidence = Evidence {
            source: "s".into(),
            scope: Scope("personal".into()),
            location: "https://github.com/o/p/pull/1".into(),
            field: "/user/login".into(),
            start: 0,
            end: 7,
            quote: "octocat".into(),
            trust_class: TrustClass::Structured,
            captured_at: Utc::now(),
            source_updated_at: None,
            original: None,
        };
        HashMap::from([(1, evidence)])
    }
    fn submit(predicate: &str, subject_type: &str, value: &str, datatype: &str) -> ClaimSubmit {
        serde_json::from_value(serde_json::json!({
            "subject": {"identifier": "github:account/1", "type": subject_type},
            "predicate": predicate,
            "object": {"kind": "literal", "value": value, "datatype": datatype},
            "extraction_rule": "r",
            "supports": [{"evidence_id": 1, "field": "/user/login", "start": 0, "end": 7}],
        }))
        .unwrap()
    }
    fn check(claim: ClaimSubmit) -> Result<ObservedClaim, String> {
        validate(claim, &delivered(), &manifest(), &vocabulary())
    }

    /// claims/R17: a rastro type claim names its subjects own vocabulary type
    #[test]
    fn a_rastro_type_claim_names_its_subjects_own_vocabulary_type() {
        assert!(
            check(submit(
                TYPE_PREDICATE,
                "x:Account",
                "x:Account",
                TYPE_DATATYPE
            ))
            .is_ok()
        );
        for wrong in [
            submit(TYPE_PREDICATE, "x:Account", "x:Item", TYPE_DATATYPE),
            submit(TYPE_PREDICATE, "x:Account", "x:Account", "string"),
            submit(TYPE_PREDICATE, "Person", "Person", TYPE_DATATYPE),
        ] {
            assert!(check(wrong).is_err());
        }
    }

    /// claims/R17: a rastro alias is a declared kind quoted exactly by its support
    #[test]
    fn a_rastro_alias_is_a_declared_kind_quoted_exactly_by_its_support() {
        let claim = check(submit(ALIAS_PREDICATE, "x:Account", "octocat", "login")).unwrap();
        assert_eq!(claim.predicate, ALIAS_PREDICATE);
        for wrong in [
            // Text the support doesn't quote.
            submit(ALIAS_PREDICATE, "x:Account", "admin", "login"),
            submit(ALIAS_PREDICATE, "x:Account", "octo", "login"),
            // A kind the vocabulary doesn't declare, or a type outside its domain.
            submit(ALIAS_PREDICATE, "x:Account", "octocat", "email"),
            submit(ALIAS_PREDICATE, "x:Item", "octocat", "login"),
        ] {
            assert!(check(wrong).is_err());
        }
    }

    /// claims/R30: a subject or object identifier outside the entity forms is rejected
    #[test]
    fn S24_an_identifier_outside_the_entity_forms_is_rejected() {
        let mut bad_subject = submit(TYPE_PREDICATE, "x:Account", "x:Account", TYPE_DATATYPE);
        bad_subject.subject.identifier = "foo:bar".into();
        assert!(
            check(bad_subject)
                .unwrap_err()
                .contains("<namespace>:<kind>/<value>")
        );
        // The object is well typed and in range; only its identifier is malformed.
        let vocabulary = Vocabulary::parse(&format!(
            "namespace = \"x\"\nversion = \"1\"\n\
             [[types]]\nid = \"x:Account\"\n{WORDS}\
             [[predicates]]\nid = \"x:knows\"\ncardinality = \"many\"\n\
             domain = [\"x:Account\"]\nrange = [\"x:Account\"]\nfreshness = \"source\"\nauthority = \"newer_comparable_observation\"\n\
             sensitivity = \"low\"\n{WORDS}"
        ))
        .unwrap();
        let knows = |object: &str| -> ClaimSubmit {
            serde_json::from_value(serde_json::json!({
                "subject": {"identifier": "github:account/1", "type": "x:Account"},
                "predicate": "x:knows",
                "object": {"kind": "entity", "identifier": object, "type": "x:Account"},
                "extraction_rule": "r",
                "supports": [{"evidence_id": 1, "field": "/user/login", "start": 0, "end": 7}],
            }))
            .unwrap()
        };
        let run = |object: &str| validate(knows(object), &delivered(), &manifest(), &vocabulary);
        assert!(run("github:account/2").is_ok());
        assert!(run("github:login").is_err());
    }

    fn typed(identifier: &str, entity_type: &str) -> ObservedClaim {
        let subject = Entity::new(identifier.into(), entity_type.into());
        let object = Object::Literal {
            value: entity_type.into(),
            datatype: TYPE_DATATYPE.into(),
        };
        ObservedClaim {
            scalar: false,
            type_ancestors: Vec::new(),
            id: claim_id(&subject.id, TYPE_PREDICATE, &object),
            subject,
            predicate: TYPE_PREDICATE.into(),
            object,
            supports: BTreeSet::new(),
        }
    }
    fn about(subject: (&str, &str), object: (&str, &str)) -> ObservedClaim {
        let subject = Entity::new(subject.0.into(), subject.1.into());
        let Entity {
            id,
            identifier,
            entity_type,
        } = Entity::new(object.0.into(), object.1.into());
        let object = Object::Entity {
            id,
            identifier,
            entity_type,
        };
        ObservedClaim {
            scalar: false,
            type_ancestors: Vec::new(),
            id: claim_id(&subject.id, "x:author", &object),
            subject,
            predicate: "x:author".into(),
            object,
            supports: BTreeSet::new(),
        }
    }

    /// claims/R5: a claim whose endpoint type is not claimed in the submission is rejected
    #[test]
    fn a_claim_whose_endpoint_type_is_not_claimed_in_the_submission_is_rejected() {
        let author = about(("i/1", "x:Item"), ("github:account/7", "x:Account"));
        let kinds = [
            typed("i/1", "x:Item"),
            typed("github:account/7", "x:Account"),
        ];
        let (kept, rejected) = ground([author.clone(), kinds[0].clone(), kinds[1].clone()].into());
        assert_eq!((kept.len(), rejected), (3, 0));
        // Either endpoint untyped here (an earlier submission's type doesn't count), or typed
        // as something else, rejects the claim; the type claims themselves stay.
        for kinds in [
            vec![kinds[0].clone()],
            vec![kinds[1].clone()],
            vec![typed("i/1", "x:Item"), typed("github:account/7", "x:Bot")],
        ] {
            let mut all = vec![author.clone()];
            all.extend(kinds.iter().cloned());
            let (kept, rejected) = ground(all);
            assert_eq!(rejected, 1);
            assert_eq!(kept.len(), kinds.len());
            assert!(kept.iter().all(|c| c.predicate == TYPE_PREDICATE));
        }
    }
}
