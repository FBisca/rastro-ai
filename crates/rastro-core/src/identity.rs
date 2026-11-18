//! Owner identity (ADR 0010): the owner-intent envelope, its resolution per domain, the claims
//! the resolution backs, and the `rastro identity` writes. Storage keeps the intents insert-only
//! and outside the rebuildable projection; this module decides what they mean.
use crate::claims::{self, ALIAS_PREDICATE, Object, TYPE_DATATYPE, TYPE_PREDICATE};
use crate::{Rastro, Scope, Snapshot, Storage};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

mod candidates;
pub(crate) use candidates::plan_answer;
pub use candidates::*;

/// The canonical encoding version of an intent row.
pub const FORMAT_VERSION: i64 = 1;
/// The version of the tagged target and payload structures.
pub const SHAPE_VERSION: u32 = 1;
/// The owner every intent of this instance belongs to.
pub const OWNER: &str = "owner";
/// The reserved entity form that names the owner's Person.
pub const ME: &str = "me";
/// The domain of the intent that mints the owner's Person.
pub const ME_DOMAIN: &str = "person/me";
pub const PERSON_TYPE: &str = "rastro:Person";
pub const HELD_BY: &str = "rastro:held_by";
pub const SAME_AS: &str = "rastro:same_as";
pub const TOKEN_OWNER: &str = "rastro:token_owner";
pub const VERIFIED_EMAIL: &str = "rastro:verified_email";
/// The alias kind of a Person's name, written `rastro:name/<name>`.
pub const NAME_KIND: &str = "name";
/// The justification kind of an intent-backed claim.
pub const OWNER_INTENT: &str = "owner_intent";
const PERSON_PREFIX: &str = "rastro:person/";
/// The prefix of the domain of an intent about one claim: `claim/<claim id>`.
const CLAIM_DOMAIN: &str = "claim/";
/// The identifier prefix of the `std:Commitment` an accepted queue item becomes.
const COMMITMENT_PREFIX: &str = "std:commitment/";
/// The type of the commitment an accepted queue item becomes, asserted by the acceptance.
const COMMITMENT_TYPE: &str = "std:Commitment";

/// The kinds the table holds. A claim intent (`confirm`, or `reject` of a claim) names a
/// [`Targets::Claim`].
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    CreatePerson,
    Name,
    Link,
    CannotLink,
    SameAs,
    Undo,
    /// Accept a queue proposal unchanged.
    Accept,
    /// Accept a queue proposal with the owner's edits.
    Edit,
    /// Reject a queue proposal, with the owner's reason; or retract a claim the owner read: it
    /// leaves every current read and `why` says it is retracted.
    Reject,
    /// Report something Rastro should have caught, about an item or not.
    Missed,
    /// Answer a queue question: the two accounts are one person, or are not. One answer, one
    /// intent, whatever it decides.
    Answer,
    /// Confirm a claim the owner read: it stays alive on the owner's word and reads `confirmed`.
    Confirm,
}
impl IntentKind {
    pub const ALL: [Self; 12] = [
        Self::CreatePerson,
        Self::Name,
        Self::Link,
        Self::CannotLink,
        Self::SameAs,
        Self::Undo,
        Self::Accept,
        Self::Edit,
        Self::Reject,
        Self::Missed,
        Self::Answer,
        Self::Confirm,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreatePerson => "create_person",
            Self::Name => "name",
            Self::Link => "link",
            Self::CannotLink => "cannot_link",
            Self::SameAs => "same_as",
            Self::Undo => "undo",
            Self::Accept => "accept",
            Self::Edit => "edit",
            Self::Reject => "reject",
            Self::Missed => "missed",
            Self::Answer => "answer",
            Self::Confirm => "confirm",
        }
    }
    /// The core validates kinds; storage keeps them as text with no SQL enum.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

/// An entity an intent names: its id, its canonical identifier and its role in the intent.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntityTarget {
    pub id: String,
    pub identifier: String,
    pub role: String,
}
/// A claim an intent reviews: its id, the revision the owner saw, the statement it said, and the
/// lifecycle state the intent binds to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimTarget {
    pub id: String,
    pub revision: String,
    /// A [`ClaimStatement`], snapshotted so a replay holds the claim without its evidence.
    pub statement: Value,
    /// A [`LifecycleBinding`]; none on a target that binds to nothing.
    pub lifecycle_binding: Option<Value>,
    /// What an `edit` fixes the claim's object to; a confirmation or a retraction has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed: Option<Object>,
}
/// What a claim says, as the owner read it: copied into the intent, because the projection that
/// held the claim is rebuilt from evidence and may no longer hold it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimStatement {
    pub subject: EntityTarget,
    pub predicate: String,
    pub object: Object,
    /// Whether a newer value of the predicate supersedes this one.
    pub scalar: bool,
}
impl ClaimStatement {
    /// The claim id this statement has: the hash of its subject, predicate and object.
    pub fn claim_id(&self) -> String {
        claims::claim_id(&self.subject.id, &self.predicate, &self.object)
    }
}
/// The lifecycle state an intent binds to (ADR 0014), versioned so the definition can change.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleBinding {
    pub version: u32,
    /// [`claims::lifecycle_fingerprint`] of the claim when the owner issued the intent.
    pub fingerprint: String,
}
impl ClaimTarget {
    /// A claim target for `claim` as the owner saw it at `revision`, bound to `fingerprint`.
    pub fn review(
        revision: String,
        statement: &ClaimStatement,
        fingerprint: String,
    ) -> Result<Self> {
        Ok(Self {
            id: statement.claim_id(),
            revision,
            statement: serde_json::to_value(statement)?,
            lifecycle_binding: Some(serde_json::to_value(LifecycleBinding {
                version: SHAPE_VERSION,
                fingerprint,
            })?),
            fixed: None,
        })
    }
    /// The same target, fixing the claim's object to `object`.
    pub fn fixing(self, object: Object) -> Self {
        Self {
            fixed: Some(object),
            ..self
        }
    }
    /// The statement, when it parses.
    pub fn statement(&self) -> Result<ClaimStatement> {
        Ok(serde_json::from_value(self.statement.clone())?)
    }
    /// The lifecycle binding, when there is one and it parses.
    pub fn binding(&self) -> Result<Option<LifecycleBinding>> {
        Ok(self
            .lifecycle_binding
            .clone()
            .map(serde_json::from_value)
            .transpose()?)
    }
}
/// The tagged target structure: an entity, a pair, a claim, or the queue item an answer names;
/// claim set is reserved.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum Targets {
    Entity {
        version: u32,
        entity: EntityTarget,
    },
    /// Two endpoints with roles, sorted by id only for a symmetric relation.
    Pair {
        version: u32,
        symmetric: bool,
        endpoints: [EntityTarget; 2],
    },
    Claim {
        version: u32,
        claim: ClaimTarget,
        anchor: String,
    },
    ClaimSet {
        version: u32,
        members: Vec<ClaimTarget>,
        anchor: String,
    },
    /// One queue proposal, named by the content-addressed id the queue read gave it, and what
    /// the owner answered about it: the reason they rejected it, the words of a missed report,
    /// and the statement they saw and, when they edited it, the one they made. An accepted
    /// proposal also carries the commitment it becomes — one `std:Commitment` of its own, so two
    /// acceptances never merge into one statement set — with the parties the item mapped to
    /// Persons and the standard terms it states, as the owner accepted them. Every answer that
    /// names an item records the scope of the item's source, so the claim and its outcome are
    /// visible only where that source is. A missed report may name no item: it can be about
    /// something that was never queued.
    Proposal {
        version: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seen: Option<StatementSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edited: Option<StatementSnapshot>,
        /// The commitment an accepted proposal becomes: the `std:Commitment` its statements are
        /// about, one per acceptance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<EntityTarget>,
        /// The Persons the item mapped to parties, so the commitment is part of their view:
        /// `about me` shows what the owner accepted.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        parties: Vec<EntityTarget>,
        /// The accepted proposal's standard statements, as the owner accepted them; a rejection
        /// or a missed report states none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        terms: Vec<StandardTerm>,
        /// The scope of the item's source: the accepted claim and the outcome are visible only
        /// to readers that see it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
    /// A queue question's answer: the item it settles, its two accounts, the verdict, and for a
    /// yes the Person that holds them. One intent both records the answer and decides what the
    /// identity graph would otherwise record as a creation, a name and two links.
    Answer {
        version: u32,
        item: String,
        endpoints: [EntityTarget; 2],
        yes: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person: Option<AnswerPerson>,
        /// The accounts the answer links to that Person, as it decided when it was written.
        /// A replay uses these, never what later intents made of the accounts.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        linked: Vec<EntityTarget>,
    },
}
/// The Person a question's yes holds its accounts by.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerPerson {
    pub entity: EntityTarget,
    /// The name this answer gives the Person it creates; none when the Person already existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether this answer creates the Person, rather than finding it already holding an
    /// account of the pair.
    pub created: bool,
}
/// One standard statement an accepted proposal holds, as the owner accepted it: a `std:`
/// predicate and the literal it carries — a party's name exactly as the source said it, or the
/// stated date. The claim it mints carries them, and the promise is not among them: the standard
/// vocabulary has no predicate for it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardTerm {
    pub predicate: String,
    pub value: String,
}
/// A proposal's statement, as the owner saw it and, when they edited it, as they made it: the
/// promise and the stated date's text, snapshotted so the record outlives the item's source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatementSnapshot {
    pub promise: String,
    /// The date as the source stated it; none when it was unknown.
    pub stated_date: Option<String>,
}
impl Targets {
    fn entity(entity: EntityTarget) -> Self {
        Self::Entity {
            version: SHAPE_VERSION,
            entity,
        }
    }
    /// A pair; a symmetric one is sorted by entity id.
    fn pair(symmetric: bool, a: EntityTarget, b: EntityTarget) -> Self {
        let endpoints = if symmetric && b.id < a.id {
            [b, a]
        } else {
            [a, b]
        };
        Self::Pair {
            version: SHAPE_VERSION,
            symmetric,
            endpoints,
        }
    }
    /// Every entity the targets name, in order. A queue item is not an entity; an answer's
    /// accounts come first, then the Person a yes holds them by.
    pub fn entities(&self) -> Vec<&EntityTarget> {
        match self {
            Self::Entity { entity, .. } => vec![entity],
            Self::Pair { endpoints, .. } => endpoints.iter().collect(),
            Self::Answer {
                endpoints, person, ..
            } => {
                let mut entities: Vec<&EntityTarget> = endpoints.iter().collect();
                if let Some(person) = person {
                    entities.push(&person.entity);
                }
                entities
            }
            Self::Claim { .. } | Self::ClaimSet { .. } => vec![],
            // A queue item is not an entity; an accepted proposal's commitment and the parties
            // it maps are, so the issuer must see them.
            Self::Proposal {
                subject, parties, ..
            } => subject.iter().chain(parties).collect(),
        }
    }
    fn version(&self) -> u32 {
        match self {
            Self::Entity { version, .. }
            | Self::Pair { version, .. }
            | Self::Claim { version, .. }
            | Self::ClaimSet { version, .. }
            | Self::Proposal { version, .. }
            | Self::Answer { version, .. } => *version,
        }
    }
    /// The claim the targets review, when they name one.
    pub fn claim(&self) -> Option<&ClaimTarget> {
        match self {
            Self::Claim { claim, .. } => Some(claim),
            _ => None,
        }
    }
    /// The queue item the targets name, when they name one.
    pub fn queue_item(&self) -> Option<&str> {
        match self {
            Self::Proposal { item, .. } => item.as_deref(),
            Self::Answer { item, .. } => Some(item),
            _ => None,
        }
    }
    pub(crate) fn endpoints(&self) -> Option<&[EntityTarget; 2]> {
        match self {
            Self::Pair { endpoints, .. } | Self::Answer { endpoints, .. } => Some(endpoints),
            _ => None,
        }
    }
    fn single(&self) -> Option<&EntityTarget> {
        match self {
            Self::Entity { entity, .. } => Some(entity),
            _ => None,
        }
    }
}
/// The commitment an accepted queue item becomes, named by the item's content-addressed id: one
/// `std:Commitment` per acceptance, so two accepted proposals never share a statement set.
pub fn commitment_target(item: &str) -> EntityTarget {
    let identifier = format!("{COMMITMENT_PREFIX}{item}");
    EntityTarget {
        id: claims::entity_id(&identifier),
        identifier,
        role: "commitment".into(),
    }
}
/// The versioned operation data.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub version: u32,
    /// A `name` intent's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
impl Payload {
    pub(crate) fn empty() -> Self {
        Self {
            version: SHAPE_VERSION,
            name: None,
        }
    }
}
/// Who issued an intent, through which surface, with which scopes: the surface's own configured
/// scopes as it issued. For the CLI, the scopes of its configured sources; a scope held only in
/// storage (a removed source's) is not listed. An empty list means no source was configured, or a
/// row written before the scopes were recorded. Audit data, not authorization: the CLI is checked
/// as seeing every scope.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IssuerContext {
    pub principal: String,
    pub surface: String,
    pub scopes: Vec<Scope>,
}
impl IssuerContext {
    /// The owner on the command line, with the scopes of its configured sources, each once and
    /// in order.
    pub fn owner_cli(sources: &[crate::Source]) -> Self {
        let scopes: BTreeSet<&String> = sources.iter().map(|s| &s.scope.0).collect();
        Self {
            principal: OWNER.into(),
            surface: "cli".into(),
            scopes: scopes.into_iter().map(|s| Scope(s.clone())).collect(),
        }
    }
}

/// One owner intent, as the insert-only table holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Intent {
    /// Portable and immutable; a retry reuses it.
    pub id: String,
    /// Acceptance order; 0 until storage accepts the intent.
    pub append_seq: i64,
    pub format_version: i64,
    pub kind: IntentKind,
    pub owner_id: String,
    pub issuer_context: IssuerContext,
    pub issued_at: DateTime<Utc>,
    pub domain_key: String,
    pub targets: Targets,
    pub payload: Payload,
    pub undo_of: Option<String>,
    pub supersedes: Option<String>,
}
impl Intent {
    /// Whether two intents are the same row apart from acceptance order: a retry.
    pub fn same_as_retry(&self, other: &Self) -> bool {
        Self {
            append_seq: 0,
            ..self.clone()
        } == Self {
            append_seq: 0,
            ..other.clone()
        }
    }
    /// The domain key its kind and targets give, for every kind but `undo` (whose domain is the
    /// undone intent's).
    pub fn expected_domain(&self) -> Option<String> {
        let pair = |prefix: &str| {
            self.targets
                .endpoints()
                .map(|[a, b]| format!("{prefix}/{}/{}", a.id, b.id))
        };
        match self.kind {
            IntentKind::CreatePerson if self.domain_key == ME_DOMAIN => Some(ME_DOMAIN.into()),
            IntentKind::CreatePerson => self.targets.single().map(|p| format!("person/{}", p.id)),
            IntentKind::Name => self
                .targets
                .single()
                .map(|p| format!("person/{}/name", p.id)),
            IntentKind::Link => pair("held_by"),
            IntentKind::CannotLink => pair("cannot_link"),
            IntentKind::SameAs => pair("same_as"),
            IntentKind::Undo => None,
            IntentKind::Reject | IntentKind::Edit if self.targets.claim().is_some() => self
                .targets
                .claim()
                .map(|claim| format!("{CLAIM_DOMAIN}{}", claim.id)),
            IntentKind::Accept | IntentKind::Edit | IntentKind::Reject | IntentKind::Answer => self
                .targets
                .queue_item()
                .map(|item| format!("queue/{item}")),
            IntentKind::Confirm => self
                .targets
                .claim()
                .map(|claim| format!("{CLAIM_DOMAIN}{}", claim.id)),
            // A missed report needs no item, so its domain names only what it is.
            IntentKind::Missed => Some(match self.targets.queue_item() {
                Some(item) => format!("queue/{item}"),
                None => "queue/missed".into(),
            }),
        }
    }
    /// The queue item this intent names, when it answers one.
    pub fn queue_item(&self) -> Option<&str> {
        self.targets.queue_item()
    }
    /// The Person this intent mints: the one a creation names, or the one an answer's yes
    /// creates. Nobody could see it before the intent, so storage skips the visibility check.
    pub fn minted_person(&self) -> Option<&EntityTarget> {
        match (&self.kind, &self.targets) {
            (IntentKind::CreatePerson, Targets::Entity { entity, .. }) => Some(entity),
            (
                IntentKind::Answer,
                Targets::Answer {
                    person:
                        Some(AnswerPerson {
                            entity,
                            created: true,
                            ..
                        }),
                    ..
                },
            ) => Some(entity),
            _ => None,
        }
    }
    /// The claim this intent corrects, when it is a confirmation, a retraction or a fix of one.
    pub fn corrects(&self) -> Option<&str> {
        match self.kind {
            IntentKind::Confirm | IntentKind::Reject | IntentKind::Edit => {
                self.targets.claim().map(|claim| claim.id.as_str())
            }
            _ => None,
        }
    }
    /// The id of the claim a fix asserts, when this intent is one.
    pub fn asserted_claim(&self) -> Option<String> {
        let claim = self.targets.claim()?;
        let statement = claim.statement().ok()?;
        let fixed = claim
            .fixed
            .as_ref()
            .filter(|_| self.kind == IntentKind::Edit)?;
        Some(claims::claim_id(
            &statement.subject.id,
            &statement.predicate,
            fixed,
        ))
    }
    /// Checks a correction's claim target: the statement hashes to the claim id, the revision
    /// seen and the lifecycle binding are there, and only a fix names the object it fixes to.
    fn check_review(&self) -> Result<()> {
        let Some(claim) = self.targets.claim() else {
            return Ok(());
        };
        let statement = claim.statement()?;
        match (&claim.fixed, self.kind) {
            (Some(fixed), IntentKind::Edit) => {
                ensure!(
                    *fixed != statement.object,
                    "an edit fixes the object to a different one"
                );
                ensure!(
                    !matches!(fixed, Object::Literal { value, .. } if value.trim().is_empty()),
                    "an edit's object is not empty"
                );
            }
            (None, IntentKind::Edit) => bail!("an edit names the object it fixes the claim to"),
            (Some(_), _) => bail!("only an edit names an object"),
            (None, _) => {}
        }
        ensure!(
            statement.claim_id() == claim.id,
            "a claim target's id is the hash of its statement"
        );
        let binding = claim.binding()?;
        ensure!(
            binding.is_some_and(|b| b.version == SHAPE_VERSION && !b.fingerprint.is_empty()),
            "a {} binds to the lifecycle state the owner saw",
            self.kind.as_str()
        );
        ensure!(
            !claim.revision.is_empty(),
            "a {} names the revision seen",
            self.kind.as_str()
        );
        Ok(())
    }
    /// Checks the row on its own: the envelope, the JSON shapes and the kind's target shape.
    /// What it says about other intents (an undo's target) is checked against the log.
    #[expect(
        clippy::too_many_lines,
        reason = "the envelope's field checks read as the schema they enforce"
    )]
    pub fn check(&self) -> Result<()> {
        let plain = |text: &str| !text.is_empty() && !text.chars().any(char::is_whitespace);
        ensure!(
            plain(&self.id),
            "an intent id is non-empty, without whitespace"
        );
        ensure!(
            self.format_version == FORMAT_VERSION,
            "unsupported intent format version {}",
            self.format_version
        );
        ensure!(!self.owner_id.is_empty(), "an intent names its owner");
        ensure!(
            !self.issuer_context.principal.is_empty() && !self.issuer_context.surface.is_empty(),
            "an intent names its issuer and surface"
        );
        ensure!(!self.domain_key.is_empty(), "an intent has a domain key");
        ensure!(
            self.targets.version() == SHAPE_VERSION && self.payload.version == SHAPE_VERSION,
            "unsupported intent target or payload version"
        );
        for entity in self.targets.entities() {
            ensure!(
                plain(&entity.id) && !entity.identifier.is_empty() && !entity.role.is_empty(),
                "an intent target has an id, an identifier and a role"
            );
            ensure!(
                entity.id == claims::entity_id(&entity.identifier),
                "an intent target's id is the hash of its identifier"
            );
        }
        match &self.targets {
            Targets::Claim { claim, anchor, .. } => {
                ensure!(&claim.id == anchor, "a claim target's anchor is the claim");
            }
            Targets::ClaimSet {
                members, anchor, ..
            } => {
                let ids: BTreeSet<_> = members.iter().map(|m| &m.id).collect();
                ensure!(
                    !members.is_empty() && ids.len() == members.len(),
                    "a claim set's members are unique"
                );
                ensure!(
                    ids.contains(anchor),
                    "a claim set's anchor is one of its members"
                );
            }
            _ => {}
        }
        if let Some([a, b]) = self.targets.endpoints() {
            ensure!(a.id != b.id, "an intent can't pair an entity with itself");
        }
        let shape_fits = match self.kind {
            IntentKind::CreatePerson | IntentKind::Name => {
                matches!(&self.targets, Targets::Entity { entity, .. } if entity.identifier.starts_with(PERSON_PREFIX))
            }
            IntentKind::Link => {
                matches!(&self.targets, Targets::Pair { symmetric: false, endpoints, .. } if endpoints[0].role == "account" && endpoints[1].role == "person")
            }
            IntentKind::CannotLink => {
                matches!(&self.targets, Targets::Pair { symmetric: true, endpoints, .. } if endpoints[0].id < endpoints[1].id)
            }
            IntentKind::SameAs => {
                matches!(&self.targets, Targets::Pair { symmetric: true, endpoints, .. } if endpoints[0].id < endpoints[1].id)
            }
            // An undo takes the targets of the intent it undoes, so the log's check of that
            // target stands for the claim a correction named.
            IntentKind::Undo => !matches!(&self.targets, Targets::ClaimSet { .. }),
            // A correction names the claim it reviewed: the statement it said, the revision the
            // owner read and the lifecycle state it binds to.
            IntentKind::Confirm | IntentKind::Reject | IntentKind::Edit
                if self.targets.claim().is_some() =>
            {
                self.check_review()?;
                true
            }
            IntentKind::Confirm => false,
            IntentKind::Accept | IntentKind::Edit | IntentKind::Reject => {
                matches!(&self.targets, Targets::Proposal { item: Some(item), .. } if plain(item))
            }
            IntentKind::Missed => match &self.targets {
                Targets::Proposal {
                    item: Some(item), ..
                } => plain(item),
                Targets::Proposal { item: None, .. } => true,
                _ => false,
            },
            IntentKind::Answer => match &self.targets {
                Targets::Answer {
                    item,
                    endpoints,
                    yes,
                    person,
                    linked,
                    ..
                } => {
                    let accounts = endpoints[0].role == "account"
                        && endpoints[1].role == "account"
                        && endpoints[0].id != endpoints[1].id;
                    // What it links is what it decided: one of its own accounts.
                    let links = (linked.iter()).all(|account| {
                        account.role == "account"
                            && endpoints.iter().any(|endpoint| endpoint.id == account.id)
                    });
                    plain(item)
                        && accounts
                        && links
                        && if *yes {
                            // A yes names the Person that holds them; a created one is named.
                            person.as_ref().is_some_and(|answer| {
                                answer.entity.role == "person"
                                    && (!answer.created || answer.name.is_some())
                            })
                        } else {
                            person.is_none() && linked.is_empty()
                        }
                }
                _ => false,
            },
        };
        ensure!(
            shape_fits,
            "a {} intent doesn't take these targets",
            self.kind.as_str()
        );
        match (self.kind, &self.payload.name) {
            (IntentKind::Name, Some(name)) => ensure!(
                crate::entity_form::is_alias_value(name) && name.len() <= claims::MAX_TERM_BYTES,
                "a name is non-empty, with no whitespace at either end"
            ),
            (IntentKind::Name, None) => bail!("a name intent carries the name"),
            (_, Some(_)) => bail!("only a name intent carries a name"),
            _ => {}
        }
        // One answer decides a whole question, so only an answer names one; an undo takes the
        // targets of the intent it undoes, exactly as it does for every other kind.
        ensure!(
            !matches!(&self.targets, Targets::Answer { .. })
                || matches!(self.kind, IntentKind::Answer | IntentKind::Undo),
            "only an answer, or the undo of one, takes a queue question"
        );
        match &self.targets {
            Targets::Proposal {
                item,
                text,
                seen,
                edited,
                subject,
                parties,
                terms,
                scope,
                ..
            } => match self.kind {
                // An accept or an edit mints the commitment it decided, so it names the entity
                // its statements are about, the terms themselves and the source's scope.
                IntentKind::Accept => {
                    ensure!(
                        seen.is_some() && edited.is_none(),
                        "an accept snapshots the proposal it saw, and no edit"
                    );
                    ensure!(
                        subject.is_some()
                            && !parties.is_empty()
                            && !terms.is_empty()
                            && scope.is_some(),
                        "an accept names the commitment it mints, its parties, its standard terms and its source's scope"
                    );
                    ensure!(
                        (terms.iter()).all(|term| {
                            term.predicate.starts_with("std:") && !term.value.trim().is_empty()
                        }),
                        "an accept's standard terms are `std:` predicates with a literal"
                    );
                }
                IntentKind::Edit => {
                    ensure!(
                        seen.is_some() && edited.is_some(),
                        "an edit snapshots the proposal it saw and the one it made"
                    );
                    ensure!(
                        subject.is_some()
                            && !parties.is_empty()
                            && !terms.is_empty()
                            && scope.is_some(),
                        "an edit names the commitment it mints, its parties, its standard terms and its source's scope"
                    );
                    ensure!(
                        (terms.iter()).all(|term| {
                            term.predicate.starts_with("std:") && !term.value.trim().is_empty()
                        }),
                        "an edit's standard terms are `std:` predicates with a literal"
                    );
                }
                // A rejection or a missed report is a decision about an item, not a commitment:
                // it states no terms and mints no claim. It names its source's scope whenever it
                // names an item, so its outcome is visible only where that source is.
                IntentKind::Reject | IntentKind::Missed => {
                    ensure!(
                        subject.is_none() && parties.is_empty() && terms.is_empty(),
                        "a {} mints no commitment",
                        self.kind.as_str()
                    );
                    ensure!(
                        item.is_some() == scope.is_some(),
                        "a {} names its source's scope exactly when it names an item",
                        self.kind.as_str()
                    );
                    ensure!(
                        text.as_ref().is_some_and(|text| !text.trim().is_empty()),
                        "a {} carries its text",
                        self.kind.as_str()
                    );
                }
                _ => ensure!(
                    seen.is_none()
                        && edited.is_none()
                        && text.is_none()
                        && subject.is_none()
                        && parties.is_empty()
                        && terms.is_empty()
                        && scope.is_none(),
                    "only a queue answer carries what the owner answered"
                ),
            },
            _ => ensure!(
                self.kind != IntentKind::Accept
                    && (!matches!(self.kind, IntentKind::Edit | IntentKind::Reject)
                        || self.targets.claim().is_some())
                    && self.kind != IntentKind::Missed,
                "a {} intent takes a queue proposal",
                self.kind.as_str()
            ),
        }
        ensure!(
            (self.kind == IntentKind::Undo) == self.undo_of.is_some(),
            "an undo, and only an undo, names the intent it undoes"
        );
        if let Some(domain) = self.expected_domain() {
            ensure!(
                self.domain_key == domain,
                "a {} intent's domain key is {domain}",
                self.kind.as_str()
            );
        }
        Ok(())
    }
}

/// Where an intent stands once every later intent is read.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    /// It contributes what its kind says.
    Effective,
    /// An undo removed its contribution.
    Undone,
    /// A cannot-link that an explicit link or merge of its pair cleared.
    Reversed,
    /// A name a newer name of the same Person replaced.
    Superseded,
}
/// A Person an intent minted.
#[derive(Clone, Debug)]
pub struct Person {
    pub entity: EntityTarget,
    pub created_by: String,
    pub me: bool,
}
/// The intents read in acceptance order, per domain: creating, naming, linking and merging never
/// overwrite one another, and an undo removes only the contribution of the intent it names.
#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub persons: BTreeMap<String, Person>,
    pub me: Option<String>,
    /// Person id to its current name and the intent that gave it.
    pub names: BTreeMap<String, (String, String)>,
    /// (account id, Person id) to the link intent.
    pub links: BTreeMap<(String, String), String>,
    /// Sorted (Person id, Person id) to the merge intent.
    pub merges: BTreeMap<(String, String), String>,
    /// Sorted endpoint ids to the cannot-link intent in force.
    pub cannot: BTreeMap<(String, String), String>,
    /// Claim id to the confirmation in force on it: the latest correction of the claim, when
    /// that is a confirmation.
    pub confirmed: BTreeMap<String, String>,
    /// Claim id to the rejection in force on it: the latest correction of the claim, when that
    /// is a rejection.
    pub retracted: BTreeMap<String, String>,
    /// Each intent's status, and the intent that gave it that status (the undo, the clearing
    /// link or merge, the newer name).
    pub status: BTreeMap<String, (IntentStatus, Option<String>)>,
    entities: BTreeMap<String, EntityTarget>,
    intents: BTreeMap<String, Intent>,
    issued: BTreeMap<String, DateTime<Utc>>,
    /// Intent ids in acceptance order.
    order: Vec<String>,
}
fn sorted(a: &str, b: &str) -> (String, String) {
    if b < a {
        (b.into(), a.into())
    } else {
        (a.into(), b.into())
    }
}
impl Resolution {
    /// Resolves intents given in acceptance order.
    #[expect(
        clippy::too_many_lines,
        reason = "resolution folds every intent's targets into the indexes in one ordered pass"
    )]
    pub fn of(intents: &[Intent]) -> Self {
        let undone: BTreeMap<&str, &str> = intents
            .iter()
            .filter(|i| i.kind == IntentKind::Undo)
            .filter_map(|i| Some((i.undo_of.as_deref()?, i.id.as_str())))
            .collect();
        let mut r = Self::default();
        for intent in intents {
            let id = intent.id.clone();
            r.issued.insert(id.clone(), intent.issued_at);
            r.intents.insert(id.clone(), intent.clone());
            r.order.push(id.clone());
            for entity in intent.targets.entities() {
                r.entities
                    .entry(entity.id.clone())
                    .or_insert_with(|| entity.clone());
            }
            if let Some(by) = undone.get(id.as_str()) {
                r.status
                    .insert(id, (IntentStatus::Undone, Some(by.to_string())));
                continue;
            }
            r.status.insert(id.clone(), (IntentStatus::Effective, None));
            match intent.kind {
                IntentKind::CreatePerson => {
                    let Some(entity) = intent.targets.single() else {
                        continue;
                    };
                    let me = intent.domain_key == ME_DOMAIN;
                    if me && r.me.is_none() {
                        r.me = Some(entity.id.clone());
                    }
                    r.persons.entry(entity.id.clone()).or_insert(Person {
                        entity: entity.clone(),
                        created_by: id,
                        me,
                    });
                }
                IntentKind::Name => {
                    let (Some(person), Some(name)) =
                        (intent.targets.single(), intent.payload.name.clone())
                    else {
                        continue;
                    };
                    if let Some((_, older)) = r.names.insert(person.id.clone(), (name, id.clone()))
                    {
                        r.status.insert(older, (IntentStatus::Superseded, Some(id)));
                    }
                }
                IntentKind::Link => {
                    let Some([account, person]) = intent.targets.endpoints() else {
                        continue;
                    };
                    r.links
                        .insert((account.id.clone(), person.id.clone()), id.clone());
                    r.clear(&sorted(&account.id, &person.id), &id);
                }
                IntentKind::SameAs => {
                    let Some([a, b]) = intent.targets.endpoints() else {
                        continue;
                    };
                    let pair = sorted(&a.id, &b.id);
                    r.merges.insert(pair.clone(), id.clone());
                    r.clear(&pair, &id);
                }
                IntentKind::CannotLink => {
                    if let Some([a, b]) = intent.targets.endpoints() {
                        r.cannot.entry(sorted(&a.id, &b.id)).or_insert(id);
                    }
                }
                IntentKind::Answer => {
                    let Targets::Answer {
                        endpoints: [a, b],
                        yes,
                        person,
                        linked,
                        ..
                    } = &intent.targets
                    else {
                        continue;
                    };
                    if !*yes {
                        r.cannot.entry(sorted(&a.id, &b.id)).or_insert(id);
                        continue;
                    }
                    let Some(answer) = person else {
                        continue;
                    };
                    let holder = answer.entity.clone();
                    if answer.created {
                        r.persons.entry(holder.id.clone()).or_insert(Person {
                            entity: holder.clone(),
                            created_by: id.clone(),
                            me: false,
                        });
                        if let Some(name) = answer.name.clone() {
                            r.names.insert(holder.id.clone(), (name, id.clone()));
                        }
                    }
                    // The accounts the answer decided to link, as it recorded them: a later
                    // intent on those accounts never changes what this answer said.
                    for account in linked {
                        r.links
                            .insert((account.id.clone(), holder.id.clone()), id.clone());
                        r.clear(&sorted(&account.id, &holder.id), &id);
                    }
                }
                // A proposal's answer is the owner's record of what they decided; its item
                // leaves the queue by that record alone and it backs no claim.
                IntentKind::Accept | IntentKind::Missed => {}
                IntentKind::Edit if intent.targets.claim().is_none() => {}
                // The latest valid correction of a claim is the one in force.
                IntentKind::Confirm => {
                    if let Some(claim) = intent.targets.claim() {
                        r.supersede_correction(&claim.id, &id);
                        r.confirmed.insert(claim.id.clone(), id);
                    }
                }
                // A fix retracts the claim it fixes, as a rejection does.
                IntentKind::Reject | IntentKind::Edit => {
                    if let Some(claim) = intent.targets.claim() {
                        r.supersede_correction(&claim.id, &id);
                        r.retracted.insert(claim.id.clone(), id);
                    }
                }
                IntentKind::Undo => {}
            }
        }
        r
    }
    /// A newer correction of a claim takes the place of the one in force: that one is superseded
    /// by it, so what it held ends where the newer one begins.
    fn supersede_correction(&mut self, claim: &str, by: &str) {
        let older = self
            .confirmed
            .remove(claim)
            .or_else(|| self.retracted.remove(claim));
        if let Some(older) = older {
            self.status
                .insert(older, (IntentStatus::Superseded, Some(by.into())));
        }
    }
    /// An explicit link or merge clears the cannot-link of its own pair, and only that one.
    fn clear(&mut self, pair: &(String, String), by: &str) {
        if let Some(refusal) = self.cannot.remove(pair) {
            self.status
                .insert(refusal, (IntentStatus::Reversed, Some(by.into())));
        }
    }
    /// Refuses an intent the ADR refuses, given the intents accepted before it: a link from a
    /// Person or to anything but a Person, a merge of entities that aren't both Persons, a name
    /// for a non-Person, a second creation of a Person or of `me`, and an undo of a missing,
    /// undone, cannot-link, undo or creation intent, or one whose domain or targets differ.
    /// Storage calls it on every intent, whoever built it.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per intent kind keeps the ADR's refusals readable side by side"
    )]
    pub fn admit(&self, intent: &Intent) -> Result<()> {
        let person = |e: &EntityTarget| self.is_person(&e.id);
        match intent.kind {
            IntentKind::CreatePerson => {
                if let Some(p) = intent.targets.single() {
                    ensure!(!person(p), "Person {} already exists", p.identifier);
                }
                if intent.domain_key == ME_DOMAIN {
                    ensure!(self.me.is_none(), "the owner's Person already exists");
                }
            }
            IntentKind::Name => {
                if let Some(p) = intent.targets.single() {
                    ensure!(
                        person(p),
                        "{} is not a Person; only a Person has a name",
                        p.identifier
                    );
                }
            }
            IntentKind::Link => {
                if let Some([account, holder]) = intent.targets.endpoints() {
                    ensure!(
                        !person(account),
                        "{} is a Person, and a Person is never held by another; merge two Persons with `rastro identity merge`",
                        account.identifier
                    );
                    ensure!(
                        person(holder),
                        "{} is not a Person; an account is held only by a Person",
                        holder.identifier
                    );
                }
            }
            IntentKind::SameAs => {
                if let Some([a, b]) = intent.targets.endpoints() {
                    match (person(a), person(b)) {
                        (true, true) => {}
                        (false, false) => bail!(
                            "accounts never merge; link an account to a Person with `rastro identity link`"
                        ),
                        (false, true) => {
                            bail!("only Persons merge, and {} is not a Person", a.identifier)
                        }
                        (true, false) => {
                            bail!("only Persons merge, and {} is not a Person", b.identifier)
                        }
                    }
                }
            }
            IntentKind::CannotLink => {}
            // An answer is judged when it is written, against the queue it names and the
            // identity graph's own rules; nothing about an earlier intent can refuse it.
            IntentKind::Answer
            | IntentKind::Accept
            | IntentKind::Edit
            | IntentKind::Reject
            | IntentKind::Missed
            | IntentKind::Confirm => {}
            IntentKind::Undo => {
                let undone = intent.undo_of.as_deref().unwrap_or_default();
                let Some(target) = self.intents.get(undone) else {
                    bail!("unknown intent: {undone}");
                };
                ensure!(
                    intent.domain_key == target.domain_key && intent.targets == target.targets,
                    "an undo takes the domain and targets of the intent it undoes"
                );
                if let Some((IntentStatus::Undone, _)) = self.status.get(undone) {
                    bail!("intent {undone} is already undone");
                }
                match target.kind {
                    IntentKind::CannotLink => bail!(
                        "a cannot-link can't be undone; only an explicit link or merge of its pair clears it"
                    ),
                    IntentKind::Undo => {
                        bail!("an undo can't be undone; issue the link or merge again")
                    }
                    IntentKind::CreatePerson => bail!("creating a Person can't be undone"),
                    IntentKind::Name | IntentKind::Link | IntentKind::SameAs => {}
                    IntentKind::Answer => {
                        let Targets::Answer { yes, person, .. } = &target.targets else {
                            bail!("an answer names the question it answered");
                        };
                        ensure!(
                            *yes,
                            "a no can't be undone, as a cannot-link can't; link the accounts instead"
                        );
                        // Undoing a yes ends what it decided; a Person it created goes with it,
                        // so no effective later intent may hold, name or merge that Person: each
                        // would be left claiming something that is no longer a Person.
                        if let Some(answer) = person
                            && answer.created
                        {
                            let person = &answer.entity;
                            let mut later: Vec<&String> = (self.links.iter())
                                .filter(|((_, holder), _)| holder == &person.id)
                                .map(|(_, by)| by)
                                .collect();
                            later.extend(
                                (self.merges.iter())
                                    .filter(|((a, b), _)| a == &person.id || b == &person.id)
                                    .map(|(_, by)| by),
                            );
                            if let Some((_, by)) = self.names.get(&person.id) {
                                later.push(by);
                            }
                            let effective = |by: &&String| {
                                by.as_str() != undone
                                    && matches!(
                                        self.status.get(by.as_str()),
                                        Some((IntentStatus::Effective, _))
                                    )
                            };
                            if let Some(by) = later.into_iter().find(effective) {
                                bail!(
                                    "{} is held, named or merged by intent {by}; undo that first",
                                    person.identifier
                                );
                            }
                        }
                    }
                    // A correction of a claim is undone with `rastro claim undo`, which takes
                    // back only that intent.
                    IntentKind::Confirm | IntentKind::Reject | IntentKind::Edit
                        if target.targets.claim().is_some() => {}
                    // A proposal's answer is a decision about one item; undoing it would bring
                    // back an item whose source may have replaced it.
                    IntentKind::Accept
                    | IntentKind::Edit
                    | IntentKind::Reject
                    | IntentKind::Missed => bail!("a proposal's answer can't be undone"),
                    IntentKind::Confirm => {
                        bail!("a confirmation can only be undone with `rastro claim undo`")
                    }
                }
            }
        }
        Ok(())
    }
    pub fn is_person(&self, id: &str) -> bool {
        self.persons.contains_key(id)
    }
    /// The claims the intents back, each with the intent that backs it: what the effective
    /// intents say, and what an undone link, merge or name, or a name a newer one replaced, said
    /// until then (its claim stays, superseded, with the time its contribution ended).
    #[expect(
        clippy::too_many_lines,
        reason = "effective and superseded claims are built in one pass over the resolution"
    )]
    pub fn claims(&self) -> Vec<IntentClaim> {
        let person_object = |id: &str| {
            let entity = &self.entities[id];
            Object::Entity {
                id: entity.id.clone(),
                identifier: entity.identifier.clone(),
                entity_type: PERSON_TYPE.into(),
            }
        };
        let mut out = Vec::new();
        let mut push = |subject: &EntityTarget,
                        predicate: &str,
                        object: Object,
                        scalar: bool,
                        intent: &String,
                        stretch: (DateTime<Utc>, Option<DateTime<Utc>>)| {
            out.push(IntentClaim {
                id: claims::claim_id(&subject.id, predicate, &object),
                subject: subject.clone(),
                predicate: predicate.into(),
                object,
                scalar,
                intent: intent.clone(),
                started_at: stretch.0,
                ended_at: stretch.1,
            });
        };
        for person in self.persons.values() {
            // An answer that creates a Person holds it while the answer holds; its type claim
            // is pushed with the answer's stretch below.
            if (self.intents.get(&person.created_by))
                .is_some_and(|creator| creator.kind == IntentKind::Answer)
            {
                continue;
            }
            let type_object = Object::Literal {
                value: PERSON_TYPE.into(),
                datatype: TYPE_DATATYPE.into(),
            };
            let created = self.issued[&person.created_by];
            push(
                &person.entity,
                TYPE_PREDICATE,
                type_object,
                true,
                &person.created_by,
                (created, None),
            );
        }
        for (person, name, intent, stretch) in self.name_stretches() {
            let object = Object::Literal {
                value: name,
                datatype: NAME_KIND.into(),
            };
            push(
                &self.entities[&person],
                ALIAS_PREDICATE,
                object,
                true,
                &intent,
                stretch,
            );
        }
        // A link or merge holds from its intent until its undo, if any.
        for id in &self.order {
            let intent = &self.intents[id];
            // A correction a newer one replaced ends where the newer one was issued.
            let ended = match self.status.get(id) {
                Some((IntentStatus::Undone, Some(by))) => Some(self.issued[by]),
                Some((IntentStatus::Superseded, Some(by))) if intent.corrects().is_some() => {
                    Some(self.issued[by])
                }
                _ => None,
            };
            let stretch = (intent.issued_at, ended);
            match (intent.kind, intent.targets.endpoints()) {
                (IntentKind::Link, Some([account, person])) => push(
                    &self.entities[&account.id],
                    HELD_BY,
                    person_object(&person.id),
                    false,
                    id,
                    stretch,
                ),
                (IntentKind::SameAs, Some([a, b])) => {
                    let (a, b) = sorted(&a.id, &b.id);
                    push(
                        &self.entities[&a],
                        SAME_AS,
                        person_object(&b),
                        false,
                        id,
                        stretch,
                    )
                }
                // One answer backs every claim it decided, from what it recorded: the Person
                // it creates exists while the answer holds, and each account it linked is held
                // by that Person over the same stretch — an undo ends them, it doesn't erase
                // that they held.
                (IntentKind::Answer, _) => {
                    let Targets::Answer {
                        yes: true,
                        person: Some(answer),
                        linked,
                        ..
                    } = &intent.targets
                    else {
                        continue;
                    };
                    if answer.created {
                        push(
                            &self.entities[&answer.entity.id],
                            TYPE_PREDICATE,
                            Object::Literal {
                                value: PERSON_TYPE.into(),
                                datatype: TYPE_DATATYPE.into(),
                            },
                            true,
                            id,
                            stretch,
                        );
                    }
                    for account in linked {
                        push(
                            &self.entities[&account.id],
                            HELD_BY,
                            person_object(&answer.entity.id),
                            false,
                            id,
                            stretch,
                        );
                    }
                }
                // A confirmation or a rejection holds the claim it reviewed from the statement it
                // snapshotted, while it stands: a rebuild keeps the claim even when its evidence
                // is gone. A rejection of a queue proposal names no claim and holds none.
                (IntentKind::Confirm | IntentKind::Reject, _) => {
                    let Some(claim) = intent.targets.claim() else {
                        continue;
                    };
                    let Ok(statement) = claim.statement() else {
                        continue;
                    };
                    push(
                        &statement.subject,
                        &statement.predicate,
                        statement.object.clone(),
                        statement.scalar,
                        id,
                        stretch,
                    );
                }
                // A fix retracts the claim it fixes, held from its snapshot like a rejection,
                // and asserts the claim with the new object: the same subject and predicate,
                // backed by this intent alone. Both halves end together, by an undo or by a
                // newer correction of the fixed claim, so a second fix replaces the first.
                (IntentKind::Edit, _) if intent.targets.claim().is_some() => {
                    let Some(claim) = intent.targets.claim() else {
                        continue;
                    };
                    let Ok(statement) = claim.statement() else {
                        continue;
                    };
                    push(
                        &statement.subject,
                        &statement.predicate,
                        statement.object.clone(),
                        statement.scalar,
                        id,
                        stretch,
                    );
                    if let Some(fixed) = &claim.fixed {
                        push(
                            &statement.subject,
                            &statement.predicate,
                            fixed.clone(),
                            statement.scalar,
                            id,
                            stretch,
                        );
                    }
                }
                _ => {}
            }
            // An accepted proposal becomes one observed commitment, typed `std:Commitment` (its anchor), and each of its standard terms,
            // as the owner accepted them, is a claim about the entity the item maps to a Person,
            // carrying the party's name or the stated date as a literal. The intent alone backs
            // them — no evidence span — and a proposal's answer can't be undone, so they hold
            // from the acceptance on.
            if matches!(intent.kind, IntentKind::Accept | IntentKind::Edit)
                && let Targets::Proposal {
                    subject: Some(subject),
                    terms,
                    ..
                } = &intent.targets
            {
                // The acceptance also asserts the commitment itself, its anchor: the terms rest on
                // it, so retracting the anchor takes every term nothing else justifies.
                push(
                    subject,
                    TYPE_PREDICATE,
                    Object::Literal {
                        value: COMMITMENT_TYPE.into(),
                        datatype: TYPE_DATATYPE.into(),
                    },
                    true,
                    id,
                    (intent.issued_at, None),
                );
                for term in terms {
                    push(
                        subject,
                        &term.predicate,
                        Object::Literal {
                            value: term.value.clone(),
                            datatype: "string".into(),
                        },
                        true,
                        id,
                        (intent.issued_at, None),
                    );
                }
            }
        }
        out
    }
    /// Each stretch during which a name intent gave its Person's name, as (Person id, name,
    /// intent id, (start, end)). Read in acceptance order: a Person's name is its newest name
    /// intent not yet undone, so a newer name ends the older one's stretch, and undoing the
    /// newer one starts a new stretch of the older at the undo, leaving a gap.
    /// Undoing a name already replaced ends nothing more: its stretch ended at the replacement.
    fn name_stretches(&self) -> Vec<NameStretch> {
        let mut stretches: Vec<NameStretch> = Vec::new();
        let mut in_force: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in &self.order {
            let intent = &self.intents[id];
            let (person, change) = match intent.kind {
                IntentKind::Name => {
                    let Some(person) = intent.targets.single() else {
                        continue;
                    };
                    (person.id.clone(), (None, Some(id.clone())))
                }
                // A yes that creates a Person names it in the same answer.
                IntentKind::Answer => match &intent.targets {
                    Targets::Answer {
                        yes: true,
                        person:
                            Some(AnswerPerson {
                                entity,
                                name: Some(_),
                                created: true,
                            }),
                        ..
                    } => (entity.id.clone(), (None, Some(id.clone()))),
                    _ => continue,
                },
                IntentKind::Undo => {
                    let Some(undone) = intent.undo_of.as_ref().and_then(|u| self.intents.get(u))
                    else {
                        continue;
                    };
                    let person = match (&undone.kind, &undone.targets) {
                        (IntentKind::Name, Targets::Entity { entity, .. }) => Some(entity),
                        (
                            IntentKind::Answer,
                            Targets::Answer {
                                person:
                                    Some(AnswerPerson {
                                        entity,
                                        created: true,
                                        ..
                                    }),
                                ..
                            },
                        ) => Some(entity),
                        _ => None,
                    };
                    let Some(person) = person else {
                        continue;
                    };
                    (person.id.clone(), (Some(undone.id.clone()), None))
                }
                _ => continue,
            };
            let names = in_force.entry(person.clone()).or_default();
            let before = names.last().cloned();
            match change {
                (Some(undone), _) => names.retain(|n| *n != undone),
                (_, Some(named)) => names.push(named),
                _ => {}
            }
            let after = names.last().cloned();
            if before == after {
                continue;
            }
            let at = intent.issued_at;
            if let Some(before) = before
                && let Some(open) = stretches
                    .iter_mut()
                    .rev()
                    .find(|s| s.2 == before && s.3.1.is_none())
            {
                open.3.1 = Some(at);
            }
            if let Some(after) = after
                && let Some(name) = self.name_given_by(&after)
            {
                stretches.push((person, name, after, (at, None)));
            }
        }
        stretches
    }
    /// The name one intent gives a Person: a name intent's payload, or a yes that creates a
    /// Person in its own answer.
    fn name_given_by(&self, id: &str) -> Option<String> {
        let intent = self.intents.get(id)?;
        match &intent.targets {
            Targets::Answer {
                person: Some(answer),
                ..
            } => answer.name.clone(),
            _ => intent.payload.name.clone(),
        }
    }
    /// The Persons a form could name among the `visible` entities: `me`, a Person's id or
    /// identifier, or any name a Person has or had (`rastro:name/<name>`); with every Person
    /// merged with them through merges whose two Persons are both visible, and every visible
    /// account any of them holds. Ids only, for a scoped read's replay roots.
    pub fn roots(&self, form: &str, visible: &BTreeSet<String>) -> BTreeSet<String> {
        let mut persons: BTreeSet<String> = BTreeSet::new();
        if form == ME {
            persons.extend(self.me.clone());
        }
        persons.extend(
            self.persons
                .values()
                .filter(|p| p.entity.id == form || p.entity.identifier == form)
                .map(|p| p.entity.id.clone()),
        );
        if let Some(name) = form.strip_prefix("rastro:name/") {
            persons.extend(
                self.intents
                    .values()
                    .filter(|i| {
                        i.kind == IntentKind::Name && i.payload.name.as_deref() == Some(name)
                    })
                    .filter_map(|i| i.targets.single().map(|p| p.id.clone())),
            );
        }
        persons.retain(|p| visible.contains(p));
        let mut pending: Vec<String> = persons.iter().cloned().collect();
        while let Some(next) = pending.pop() {
            for (a, b) in self.merges.keys() {
                let other = if *a == next {
                    b
                } else if *b == next {
                    a
                } else {
                    continue;
                };
                if visible.contains(other) && persons.insert(other.clone()) {
                    pending.push(other.clone());
                }
            }
        }
        let accounts: Vec<String> = self
            .links
            .keys()
            .filter(|(a, p)| persons.contains(p) && visible.contains(a))
            .map(|(a, _)| a.clone())
            .collect();
        persons.extend(accounts);
        persons
    }
}
/// The core's supertype of every account; a vocabulary declares which of its types are accounts
/// by naming it as their `supertype` (decisions/vocabulary.md §27).
pub const ACCOUNT_TYPE: &str = rastro_plugin_sdk::ACCOUNT_TYPE;

/// One current type of an entity, as the account rule judges it: its value, the supertypes its
/// claim carries, and whether the issuer can see a claim giving it.
#[derive(Clone, Debug)]
pub struct SeenType {
    pub value: String,
    pub ancestors: Vec<String>,
    pub visible: bool,
}
/// Refuses holding an entity that isn't typed as an account: the account side of a link has a
/// current type, and each of its current types is an account (has `rastro:Account` among its
/// supertypes). An entity two sources type differently, an account and something else, is not
/// held while they disagree. Judged on every current type, but a refusal names only types the
/// issuer can see. Storage runs it when a link intent is issued; a replay keeps links already
/// accepted. Automatic links are to use the same rule.
pub fn admit_account(identifier: &str, types: &[SeenType]) -> Result<()> {
    const ONLY: &str = "only an account is held by a Person";
    const HINT: &str = "if its vocabulary changed since it was read, run `rastro sync` or \
        `rastro rebuild` first";
    ensure!(!types.is_empty(), "{identifier} has no type; {ONLY}");
    let others: Vec<&SeenType> = types
        .iter()
        .filter(|t| !t.ancestors.iter().any(|a| a == ACCOUNT_TYPE))
        .collect();
    if others.is_empty() {
        return Ok(());
    }
    if others.iter().any(|t| !t.visible) {
        bail!("{identifier} is not typed as an account; {ONLY} ({HINT})");
    }
    let named: BTreeSet<&str> = others.iter().map(|t| t.value.as_str()).collect();
    bail!(
        "{identifier} is a {}, which no vocabulary declares an account; {ONLY} ({HINT})",
        named.into_iter().collect::<Vec<_>>().join(" and a ")
    )
}

/// A stretch of a Person's name: (Person id, name, intent id, (start, end)).
type NameStretch = (
    String,
    String,
    String,
    (DateTime<Utc>, Option<DateTime<Utc>>),
);
/// A claim an owner intent backs. Its justification is of kind `owner_intent`; while any of its
/// intents holds it is `observed`, and once every one has ended it is `superseded`.
#[derive(Clone, Debug)]
pub struct IntentClaim {
    pub id: String,
    pub subject: EntityTarget,
    pub predicate: String,
    pub object: Object,
    pub scalar: bool,
    pub intent: String,
    /// When this stretch of the intent's contribution began: its issue, or, for a name restored
    /// by undoing a newer one, that undo.
    pub started_at: DateTime<Utc>,
    /// When it ended: an undo, or the newer name that replaced it.
    pub ended_at: Option<DateTime<Utc>>,
}

/// Sixteen random bytes in lowercase hex: the random part of a Person id or an intent id.
fn random_hex() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn mint_person() -> Result<EntityTarget> {
    let identifier = format!("{PERSON_PREFIX}{}", random_hex()?);
    Ok(EntityTarget {
        id: claims::entity_id(&identifier),
        identifier,
        role: "person".into(),
    })
}
pub(crate) fn new_intent(
    kind: IntentKind,
    domain_key: String,
    targets: Targets,
    payload: Payload,
    undo_of: Option<String>,
) -> Result<Intent> {
    let intent = Intent {
        id: format!("i_{}", random_hex()?),
        append_seq: 0,
        format_version: FORMAT_VERSION,
        kind,
        owner_id: OWNER.into(),
        issuer_context: IssuerContext::owner_cli(&[]),
        // Stored to the millisecond, so a retry compares equal to the stored row.
        issued_at: chrono::SubsecRound::trunc_subsecs(crate::coverage::now()?, 3),
        domain_key,
        targets,
        payload,
        undo_of,
        supersedes: None,
    };
    let domain = intent.expected_domain();
    let intent = Intent {
        domain_key: domain.unwrap_or(intent.domain_key.clone()),
        ..intent
    };
    intent.check()?;
    Ok(intent)
}

/// What an identity write accepted: the intents it stored, in acceptance order (none when the
/// write was already in force), and the Person it concerns.
#[derive(Clone, Debug)]
pub struct Issued {
    pub intents: Vec<Intent>,
    pub person: Option<EntityTarget>,
    pub created: bool,
}
/// The entity a form names, with `role`: `me`, or a form `about` accepts, resolved unscoped.
pub(crate) fn resolve(
    view: &dyn Snapshot,
    resolution: &Resolution,
    form: &str,
    role: &str,
) -> Result<EntityTarget> {
    crate::entity_form::check_reference(form).map_err(anyhow::Error::msg)?;
    let found = if form == ME {
        resolution
            .me
            .as_ref()
            .map(|id| (id.clone(), resolution.persons[id].entity.identifier.clone()))
    } else {
        view.resolve_entity(form)?
    };
    let Some((id, identifier)) = found else {
        if form == ME {
            bail!("there is no `me` yet; run `rastro identity me` first");
        }
        bail!("unknown entity: {form}");
    };
    Ok(EntityTarget {
        id,
        identifier,
        role: role.into(),
    })
}
fn person(resolution: &Resolution, id: &str) -> Option<EntityTarget> {
    resolution.persons.get(id).map(|p| p.entity.clone())
}

impl<S: Storage> Rastro<S> {
    /// Mints the owner's Person once; a second run returns the same Person and records nothing.
    pub fn identity_me(&mut self) -> Result<Issued> {
        self.issue(|_, resolution| {
            if let Some(me) = &resolution.me {
                return Ok((vec![], person(resolution, me), false));
            }
            let me = mint_person()?;
            let intent = new_intent(
                IntentKind::CreatePerson,
                ME_DOMAIN.into(),
                Targets::entity(me.clone()),
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], Some(me), true))
        })
    }
    /// Mints a Person and names it, in two intents accepted together.
    pub fn create_person(&mut self, name: &str) -> Result<Issued> {
        self.issue(|_, _| {
            let person = mint_person()?;
            let create = new_intent(
                IntentKind::CreatePerson,
                String::new(),
                Targets::entity(person.clone()),
                Payload::empty(),
                None,
            )?;
            let named = new_intent(
                IntentKind::Name,
                String::new(),
                Targets::entity(person.clone()),
                Payload {
                    version: SHAPE_VERSION,
                    name: Some(name.into()),
                },
                None,
            )?;
            Ok((vec![create, named], Some(person), true))
        })
    }
    /// Links an account to a Person: the account is held by the Person. An explicit link clears
    /// a cannot-link of the same pair. A link already in force records nothing.
    pub fn link(&mut self, account: &str, holder: &str) -> Result<Issued> {
        self.issue(|view, resolution| {
            let account_target = resolve(view, resolution, account, "account")?;
            let person_target = resolve(view, resolution, holder, "person")?;
            let pair = (account_target.id.clone(), person_target.id.clone());
            if resolution.links.contains_key(&pair) {
                return Ok((vec![], Some(person_target), false));
            }
            let intent = new_intent(
                IntentKind::Link,
                String::new(),
                Targets::pair(false, account_target, person_target.clone()),
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], Some(person_target), false))
        })
    }
    /// Merges two Persons. Accounts never merge. A merge already in force records nothing.
    pub fn merge(&mut self, one: &str, other: &str) -> Result<Issued> {
        self.issue(|view, resolution| {
            let a = resolve(view, resolution, one, "person")?;
            let b = resolve(view, resolution, other, "person")?;
            if resolution.merges.contains_key(&sorted(&a.id, &b.id)) {
                return Ok((vec![], None, false));
            }
            let intent = new_intent(
                IntentKind::SameAs,
                String::new(),
                Targets::pair(true, a, b),
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], None, false))
        })
    }
    /// Undoes one earlier intent, removing only its contribution. Undoing a link or a merge also
    /// records a cannot-link for its pair.
    /// Storage refuses what can't be undone (`Resolution::admit`).
    pub fn undo(&mut self, intent_id: &str) -> Result<Issued> {
        self.issue(|_, resolution| {
            let Some(target) = resolution.intents.get(intent_id) else {
                bail!("unknown intent: {intent_id}");
            };
            ensure!(
                target.kind == IntentKind::Undo || target.targets.claim().is_none(),
                "intent {intent_id} corrects a claim; undo it with `rastro claim undo {intent_id}`"
            );
            let mut intents = vec![new_intent(
                IntentKind::Undo,
                target.domain_key.clone(),
                target.targets.clone(),
                Payload::empty(),
                Some(target.id.clone()),
            )?];
            if let (IntentKind::Link | IntentKind::SameAs, Some([a, b])) =
                (target.kind, target.targets.endpoints())
            {
                intents.push(new_intent(
                    IntentKind::CannotLink,
                    String::new(),
                    Targets::pair(true, a.clone(), b.clone()),
                    Payload::empty(),
                    None,
                )?);
            }
            Ok((intents, None, false))
        })
    }
    /// Undoes one correction of a claim, removing only its contribution: the claim reads as the
    /// latest correction still in force says, or as its evidence does when none is. Undoing an
    /// edit takes back both the retraction and the claim it asserted. Refused for an intent that
    /// corrects no claim, naming `rastro identity undo`, and for one already undone.
    pub fn undo_claim(&mut self, intent_id: &str) -> Result<Issued> {
        self.issue(|_, resolution| {
            let Some(target) = resolution.intents.get(intent_id) else {
                bail!("unknown intent: {intent_id}");
            };
            ensure!(
                target.targets.claim().is_some(),
                "intent {intent_id} corrects no claim; undo it with `rastro identity undo {intent_id}`"
            );
            let undo = new_intent(
                IntentKind::Undo,
                target.domain_key.clone(),
                target.targets.clone(),
                Payload::empty(),
                Some(target.id.clone()),
            )?;
            Ok((vec![undo], None, false))
        })
    }
    /// Confirms a claim the owner read at revision `seen`: one `confirm` intent that binds to the
    /// claim's lifecycle state and holds the claim from the statement the owner saw. Refused when
    /// the claim's current revision is not `seen`, naming the claim and its current revision.
    /// Confirming a claim already confirmed at that revision records nothing.
    pub fn confirm_claim(&mut self, claim: &str, seen: &str) -> Result<Issued> {
        self.review_claim(IntentKind::Confirm, claim, seen, None)
    }
    /// Retracts a claim the owner read at revision `seen`: one `reject` intent that binds to the
    /// claim's lifecycle state. The claim leaves every current read in every scope that sees it,
    /// and `why` says it is retracted. Refused when the claim's current revision is not `seen`,
    /// naming the claim and its current revision, and when an owner `link` (or merge, or answer)
    /// made it: that is undone with `rastro identity undo`, naming the intent. Retracting a claim
    /// already retracted records nothing.
    pub fn reject_claim(&mut self, claim: &str, seen: &str) -> Result<Issued> {
        self.review_claim(IntentKind::Reject, claim, seen, None)
    }
    /// Fixes the object of a claim the owner read at revision `seen`: one `edit` intent that
    /// retracts the claim and asserts the claim with the same subject and predicate and `object`,
    /// backed by that intent alone. `object` is read against the predicate's range: an entity of a
    /// type in it, or a literal of a datatype in it. Refused, with nothing recorded, when the
    /// object is outside the range (naming the predicate and its range), when the claim changed
    /// since `seen`, and for a claim an owner `link` made. Fixing a claim to the object it has
    /// records nothing.
    pub fn edit_claim(&mut self, claim: &str, seen: &str, object: &str) -> Result<Issued> {
        let vocabularies = self.claim_vocabularies(claim)?;
        let fix = Fix {
            vocabularies,
            object,
        };
        self.review_claim(IntentKind::Edit, claim, seen, Some(fix))
    }
    /// The vocabularies that define the predicate of `claim`: those of the extractors that read
    /// it, when they are installed, and the standard one the core bundles.
    fn claim_vocabularies(&self, claim: &str) -> Result<Vec<rastro_plugin_sdk::Vocabulary>> {
        // A claim an earlier fix asserted has no evidence of its own: its vocabulary is that of
        // the claim the fix replaced, and so on back to the one evidence read.
        let extractors = self.storage.snapshot(|view| {
            let log = view.intents()?;
            let options = crate::knowledge::WhyOptions::default();
            let mut extractors = BTreeSet::new();
            let mut visited = BTreeSet::new();
            let mut next = Some(claim.to_string());
            while let Some(id) = next.take() {
                if !visited.insert(id.clone()) {
                    break;
                }
                let Some(explained) = view.explain(&id, &options)? else {
                    ensure!(visited.len() > 1, "unknown claim: {claim}");
                    break;
                };
                extractors.extend(
                    (explained.supports.iter())
                        .filter(|support| support.justification_kind == "provenance")
                        .map(|support| support.extractor.clone()),
                );
                next = (log.iter().rev())
                    .find(|intent| intent.asserted_claim().as_deref() == Some(id.as_str()))
                    .and_then(|intent| intent.corrects().map(String::from));
            }
            Ok(extractors)
        })?;
        let mut vocabularies = vec![crate::commitments::vocabulary::standard()?];
        for extractor in extractors {
            if let Ok(plugin) = crate::host::discover(&self.locations, &extractor, "extractor")
                && let Ok(vocabulary) = crate::host::vocabulary(&plugin)
            {
                vocabularies.push(vocabulary);
            }
        }
        Ok(vocabularies)
    }
    /// One correction of one claim, as `kind` says: the claim as the owner saw it at `seen`.
    fn review_claim(
        &mut self,
        kind: IntentKind,
        claim: &str,
        seen: &str,
        fix: Option<Fix>,
    ) -> Result<Issued> {
        self.issue(|view, resolution| {
            let explained = view
                .explain(claim, &crate::knowledge::WhyOptions::default())?
                .ok_or_else(|| anyhow::anyhow!("unknown claim: {claim}"))?;
            let current = explained.claim.claim;
            ensure!(
                current.revision == seen,
                "claim {} changed since you saw it (now {}); read it again with `rastro why {}`",
                current.id,
                current.revision,
                current.id
            );
            let (held, in_force) = match kind {
                IntentKind::Reject => ("retracted", &resolution.retracted),
                _ => ("confirmed", &resolution.confirmed),
            };
            if kind != IntentKind::Edit
                && current.status == held
                && in_force.contains_key(&current.id)
            {
                return Ok((vec![], None, false));
            }
            if matches!(kind, IntentKind::Reject | IntentKind::Edit) {
                // The owner's own link, merge or answer is not a reading to retract or fix: it is
                // undone, which also records the cannot-link that keeps it from coming back.
                let made = explained.supports.iter().find(|support| {
                    support.current
                        && support.justification_kind == OWNER_INTENT
                        && matches!(support.extraction_rule.as_str(), "link" | "same_as" | "answer")
                });
                if let Some((support, intent)) = made.and_then(|s| Some((s, s.intent.as_ref()?))) {
                    bail!(
                        "claim {} was made by your {} intent {}; undo it with `rastro identity undo {}`",
                        current.id,
                        support.extraction_rule,
                        intent.id,
                        intent.id
                    );
                }
            }
            let state = view
                .claim_lifecycle(&current.id)?
                .ok_or_else(|| anyhow::anyhow!("unknown claim: {claim}"))?;
            let statement = ClaimStatement {
                subject: EntityTarget {
                    id: current.subject.id,
                    identifier: current.subject.identifier,
                    role: "subject".into(),
                },
                predicate: current.predicate,
                object: current.object,
                scalar: state.scalar,
            };
            let mut review = ClaimTarget::review(current.revision, &statement, state.fingerprint)?;
            if let Some(fix) = fix {
                let fixed = fix.object_for(view, &statement)?;
                if fixed == statement.object {
                    return Ok((vec![], None, false));
                }
                let asserted = claims::claim_id(&statement.subject.id, &statement.predicate, &fixed);
                // A claim an earlier fix made and a later one replaced may be asserted again.
                ensure!(
                    view.explain(&asserted, &crate::knowledge::WhyOptions::default())?
                        .is_none_or(|existing| existing.claim.claim.status == "superseded"),
                    "claim {asserted} already says that; confirm it with `rastro claim confirm` instead"
                );
                review = review.fixing(fixed);
            }
            let id = review.id.clone();
            let intent = new_intent(
                kind,
                String::new(),
                Targets::Claim {
                    version: SHAPE_VERSION,
                    claim: review,
                    anchor: id,
                },
                Payload::empty(),
                None,
            )?;
            Ok((vec![intent], None, false))
        })
    }
    /// Every intent naming the entity, in acceptance order, with where each stands.
    pub fn identity_history(&self, entity: &str) -> Result<History> {
        crate::entity_form::check_reference(entity).map_err(anyhow::Error::msg)?;
        self.storage.snapshot(|view| {
            let log = view.intents()?;
            let resolution = Resolution::of(&log);
            let found = if entity == ME {
                resolution
                    .me
                    .as_ref()
                    .map(|id| (id.clone(), resolution.persons[id].entity.identifier.clone()))
            } else {
                view.resolve_entity(entity)?.or_else(|| {
                    resolution
                        .entities
                        .values()
                        .find(|e| e.id == entity || e.identifier == entity)
                        .map(|e| (e.id.clone(), e.identifier.clone()))
                })
            };
            let Some((id, identifier)) = found else {
                return Ok(History {
                    entity: None,
                    intents: vec![],
                });
            };
            let intents = log
                .iter()
                .filter(|i| i.targets.entities().iter().any(|e| e.id == id))
                .map(|i| IntentView::of(i, &resolution))
                .collect();
            Ok(History {
                entity: Some(HistoryEntity { id, identifier }),
                intents,
            })
        })
    }
    pub(crate) fn issue(
        &mut self,
        plan: impl FnOnce(
            &dyn Snapshot,
            &Resolution,
        ) -> Result<(Vec<Intent>, Option<EntityTarget>, bool)>,
    ) -> Result<Issued> {
        // The CLI is the owner's own surface: it sees every scope, and records the scopes it is
        // configured with as the issuer's.
        let issuer = IssuerContext::owner_cli(&self.config.sources);
        let (intents, (person, created)) = self.storage.issue_intents(&[], |view| {
            let resolution = Resolution::of(&view.intents()?);
            let (mut intents, person, created) = plan(view, &resolution)?;
            for intent in &mut intents {
                intent.issuer_context = issuer.clone();
            }
            Ok((intents, (person, created)))
        })?;
        Ok(Issued {
            intents,
            person,
            created,
        })
    }
}

/// What an edit fixes a claim to: the words the owner typed, and the vocabularies that say what
/// the predicate takes.
struct Fix<'a> {
    vocabularies: Vec<rastro_plugin_sdk::Vocabulary>,
    object: &'a str,
}
impl Fix<'_> {
    /// The object the words name for the claim's predicate; an error names the predicate and its
    /// range.
    fn object_for(&self, view: &dyn Snapshot, statement: &ClaimStatement) -> Result<Object> {
        let entity = match crate::entity_form::check_reference(self.object) {
            Ok(()) if self.object != ME => view.resolve_entity(self.object)?,
            _ => None,
        };
        let entity = entity
            .map(|(id, identifier)| -> Result<claims::FoundEntity> {
                let types = view
                    .about(&id, &crate::knowledge::AboutOptions::default())?
                    .into_iter()
                    .filter(|c| c.claim.subject.id == id && c.claim.predicate == TYPE_PREDICATE)
                    .filter_map(|c| match c.claim.object {
                        Object::Literal { value, .. } => Some(value),
                        Object::Entity { .. } => None,
                    })
                    .collect();
                Ok(claims::FoundEntity {
                    id,
                    identifier,
                    types,
                })
            })
            .transpose()?;
        claims::fixed_object(
            &self.vocabularies,
            &statement.predicate,
            &statement.object,
            self.object,
            entity,
        )
        .map_err(anyhow::Error::msg)
    }
}

/// An intent as `rastro identity` shows it.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct IntentView {
    pub id: String,
    /// Acceptance order.
    pub append_seq: i64,
    pub kind: IntentKind,
    pub issued_at: DateTime<Utc>,
    pub issuer: IssuerContext,
    pub domain_key: String,
    /// The versioned, tagged target structure, as stored.
    pub targets: Value,
    /// The versioned operation data, as stored.
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    pub status: IntentStatus,
    /// The intent that gave it its status (the undo, the clearing link or merge, the newer name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_by: Option<String>,
}
impl IntentView {
    pub fn of(intent: &Intent, resolution: &Resolution) -> Self {
        let (status, status_by) = resolution
            .status
            .get(&intent.id)
            .cloned()
            .unwrap_or((IntentStatus::Effective, None));
        Self {
            id: intent.id.clone(),
            append_seq: intent.append_seq,
            kind: intent.kind,
            issued_at: intent.issued_at,
            issuer: intent.issuer_context.clone(),
            domain_key: intent.domain_key.clone(),
            targets: serde_json::to_value(&intent.targets).unwrap_or(Value::Null),
            payload: serde_json::to_value(&intent.payload).unwrap_or(Value::Null),
            undo_of: intent.undo_of.clone(),
            supersedes: intent.supersedes.clone(),
            status,
            status_by,
        }
    }
}
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct HistoryEntity {
    pub id: String,
    pub identifier: String,
}
/// The intents naming one entity, in acceptance order.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct History {
    /// The entity, or none when the form names nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<HistoryEntity>,
    pub intents: Vec<IntentView>,
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn intent(kind: IntentKind, targets: Targets, undo_of: Option<&str>, id: &str) -> Intent {
        let mut intent = Intent {
            id: id.into(),
            append_seq: 0,
            format_version: FORMAT_VERSION,
            kind,
            owner_id: OWNER.into(),
            issuer_context: IssuerContext::owner_cli(&[]),
            issued_at: "2026-11-05T00:00:00Z".parse().unwrap(),
            domain_key: String::new(),
            targets,
            payload: Payload::empty(),
            undo_of: undo_of.map(Into::into),
            supersedes: None,
        };
        intent.domain_key = intent.expected_domain().unwrap_or_default();
        intent
    }
    fn target(identifier: &str, role: &str) -> EntityTarget {
        EntityTarget {
            id: claims::entity_id(identifier),
            identifier: identifier.into(),
            role: role.into(),
        }
    }

    /// A yes answered in the queue holds the accounts it recorded, whatever a later undo did to
    /// the links on those accounts: a replay never re-decides the answer.
    #[test]
    fn an_answers_yes_holds_the_accounts_it_recorded_after_a_link_is_undone() {
        let p = target("rastro:person/1", "person");
        let a = target("github:account/1", "account");
        let b = target("linear:user/u-1", "account");
        let create = intent(
            IntentKind::CreatePerson,
            Targets::entity(p.clone()),
            None,
            "i_1",
        );
        let link = intent(
            IntentKind::Link,
            Targets::pair(false, a.clone(), p.clone()),
            None,
            "i_2",
        );
        let mut undo = intent(IntentKind::Undo, link.targets.clone(), Some("i_2"), "i_3");
        undo.domain_key = link.domain_key.clone();
        let answer = intent(
            IntentKind::Answer,
            Targets::Answer {
                version: SHAPE_VERSION,
                item: "q_1".into(),
                endpoints: [a.clone(), b.clone()],
                yes: true,
                person: Some(AnswerPerson {
                    entity: p,
                    name: None,
                    created: false,
                }),
                linked: vec![a, b],
            },
            None,
            "i_4",
        );
        let resolution = Resolution::of(&[create, link, undo, answer]);
        let claims = resolution.claims();
        let mut held: Vec<&str> = (claims.iter())
            .filter(|claim| claim.predicate == HELD_BY && claim.ended_at.is_none())
            .map(|claim| claim.subject.identifier.as_str())
            .collect();
        held.sort();
        assert_eq!(held, ["github:account/1", "linear:user/u-1"]);
    }

    /// identity/R4: an undo removes only its intents contribution and a link clears a cannot link
    #[test]
    fn an_undo_removes_only_its_intents_contribution_and_a_link_clears_a_cannot_link() {
        let p = target("rastro:person/1", "person");
        let a = target("github:account/1", "account");
        let create = intent(
            IntentKind::CreatePerson,
            Targets::entity(p.clone()),
            None,
            "i_1",
        );
        let link = intent(
            IntentKind::Link,
            Targets::pair(false, a.clone(), p.clone()),
            None,
            "i_2",
        );
        let mut undo = intent(IntentKind::Undo, link.targets.clone(), Some("i_2"), "i_3");
        undo.domain_key = link.domain_key.clone();
        let refusal = intent(
            IntentKind::CannotLink,
            Targets::pair(true, a.clone(), p.clone()),
            None,
            "i_4",
        );
        let r = Resolution::of(&[create.clone(), link.clone(), undo.clone(), refusal.clone()]);
        assert!(r.links.is_empty());
        assert_eq!(r.persons.len(), 1);
        assert_eq!(r.cannot.len(), 1);
        assert_eq!(r.status["i_2"], (IntentStatus::Undone, Some("i_3".into())));
        let again = intent(
            IntentKind::Link,
            Targets::pair(false, a.clone(), p.clone()),
            None,
            "i_5",
        );
        let r = Resolution::of(&[create, link, undo, refusal, again]);
        assert_eq!(r.links.len(), 1);
        assert!(r.cannot.is_empty());
        assert_eq!(
            r.status["i_4"],
            (IntentStatus::Reversed, Some("i_5".into()))
        );
        for i in ["i_1", "i_2", "i_3", "i_4", "i_5"] {
            assert!(r.status.contains_key(i));
        }
    }

    /// identity/R6: rows are checked on their own
    #[test]
    fn rows_are_checked_on_their_own() {
        let p = target("rastro:person/1", "person");
        let a = target("github:account/1", "account");
        assert!(
            intent(
                IntentKind::CreatePerson,
                Targets::entity(p.clone()),
                None,
                "i_1"
            )
            .check()
            .is_ok()
        );
        // A link's endpoints keep their roles, account first.
        let reversed = intent(
            IntentKind::Link,
            Targets::Pair {
                version: SHAPE_VERSION,
                symmetric: false,
                endpoints: [p.clone(), a.clone()],
            },
            None,
            "i_2",
        );
        assert!(reversed.check().is_err());
        // A symmetric pair is sorted.
        let unsorted = Targets::Pair {
            version: SHAPE_VERSION,
            symmetric: true,
            endpoints: if a.id < p.id {
                [p.clone(), a.clone()]
            } else {
                [a.clone(), p.clone()]
            },
        };
        assert!(
            intent(IntentKind::CannotLink, unsorted, None, "i_3")
                .check()
                .is_err()
        );
        // A target id that isn't its identifier's hash, an empty id, a name without one.
        let mut forged = p.clone();
        forged.id = claims::entity_id("rastro:person/2");
        assert!(
            intent(
                IntentKind::CreatePerson,
                Targets::entity(forged),
                None,
                "i_4"
            )
            .check()
            .is_err()
        );
        assert!(
            intent(
                IntentKind::CreatePerson,
                Targets::entity(p.clone()),
                None,
                ""
            )
            .check()
            .is_err()
        );
        assert!(
            intent(IntentKind::Name, Targets::entity(p.clone()), None, "i_5")
                .check()
                .is_err()
        );
        // Claim targets are reserved: unique members and an anchor among them, and no identity
        // kind takes one.
        let member = |id: &str| ClaimTarget {
            id: id.into(),
            revision: "r".into(),
            statement: serde_json::json!({}),
            lifecycle_binding: None,
            fixed: None,
        };
        let set = |members: Vec<ClaimTarget>, anchor: &str| Targets::ClaimSet {
            version: SHAPE_VERSION,
            members,
            anchor: anchor.into(),
        };
        for targets in [
            set(vec![member("c_1"), member("c_1")], "c_1"),
            set(vec![member("c_1")], "c_2"),
            set(vec![member("c_1")], "c_1"),
        ] {
            assert!(
                intent(IntentKind::Undo, targets, Some("i_1"), "i_6")
                    .check()
                    .is_err()
            );
        }
    }

    /// identity/R32: only an entity whose current types are all accounts may be held; an untyped one, a
    /// pull request, a repository or a bot may not, and the refusal names why
    #[test]
    fn only_an_entity_typed_as_an_account_may_be_held() {
        let typed = |types: &[(&str, &[&str], bool)]| -> Vec<SeenType> {
            types
                .iter()
                .map(|(t, up, visible)| SeenType {
                    value: t.to_string(),
                    ancestors: up.iter().map(|u| u.to_string()).collect(),
                    visible: *visible,
                })
                .collect()
        };
        for ok in [
            typed(&[("code:Account", &[ACCOUNT_TYPE], true)]),
            typed(&[("tracker:Account", &[ACCOUNT_TYPE], false)]),
            typed(&[("x:Admin", &["x:Account", ACCOUNT_TYPE], true)]),
        ] {
            assert!(admit_account("e", &ok).is_ok(), "{ok:?}");
        }
        for (types, reason) in [
            (typed(&[]), "has no type"),
            (
                typed(&[("code:PullRequest", &[], true)]),
                "is a code:PullRequest",
            ),
            (
                typed(&[("code:Repository", &[], true)]),
                "is a code:Repository",
            ),
            (typed(&[("tracker:Bot", &[], true)]), "is a tracker:Bot"),
            // An account another source calls a bot is not held while they disagree.
            (
                typed(&[
                    ("code:Account", &[ACCOUNT_TYPE], true),
                    ("code:Bot", &[], true),
                ]),
                "is a code:Bot",
            ),
            // A type the issuer can't see is never named.
            (
                typed(&[
                    ("code:Account", &[ACCOUNT_TYPE], true),
                    ("code:Bot", &[], false),
                ]),
                "is not typed as an account",
            ),
        ] {
            let error = admit_account("https://github.com/o/p/pull/1", &types)
                .unwrap_err()
                .to_string();
            assert!(error.contains(reason), "{error}");
            assert!(
                error.contains("only an account is held by a Person"),
                "{error}"
            );
            assert!(error.contains("https://github.com/o/p/pull/1"), "{error}");
            assert!(
                !error.contains("code:Bot") || reason.contains("code:Bot"),
                "{error}"
            );
            // An account read before its vocabulary declared it one gets a way out.
            if reason != "has no type" {
                assert!(
                    error.contains("rastro sync") && error.contains("rastro rebuild"),
                    "{error}"
                );
            }
        }
    }

    /// identity/R2: the newest name wins and undoing it restores the older
    #[test]
    fn the_newest_name_wins_and_undoing_it_restores_the_older() {
        let p = target("rastro:person/1", "person");
        let name = |value: &str, id: &str| {
            let mut i = intent(IntentKind::Name, Targets::entity(p.clone()), None, id);
            i.payload.name = Some(value.into());
            i
        };
        let first = name("Ana", "i_1");
        let second = name("Ana Silva", "i_2");
        let r = Resolution::of(&[first.clone(), second.clone()]);
        assert_eq!(r.names[&p.id].0, "Ana Silva");
        assert_eq!(r.status["i_1"].0, IntentStatus::Superseded);
        let mut undo = intent(IntentKind::Undo, second.targets.clone(), Some("i_2"), "i_3");
        undo.domain_key = second.domain_key.clone();
        let r = Resolution::of(&[first, second, undo]);
        assert_eq!(r.names[&p.id].0, "Ana");
        assert_eq!(r.status["i_1"].0, IntentStatus::Effective);
    }
}
