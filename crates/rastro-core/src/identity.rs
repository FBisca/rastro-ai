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

/// The kinds this slice enables. Slice 7 adds claim and claim-set kinds to the same table.
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
}
impl IntentKind {
    pub const ALL: [Self; 6] = [
        Self::CreatePerson,
        Self::Name,
        Self::Link,
        Self::CannotLink,
        Self::SameAs,
        Self::Undo,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreatePerson => "create_person",
            Self::Name => "name",
            Self::Link => "link",
            Self::CannotLink => "cannot_link",
            Self::SameAs => "same_as",
            Self::Undo => "undo",
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
/// A claim a slice-7 intent reviews. Reserved: no identity kind targets a claim.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimTarget {
    pub id: String,
    pub revision: String,
    pub statement: Value,
    pub lifecycle_binding: Option<Value>,
}
/// The tagged target structure. Entity and pair are used now; claim and claim set are reserved.
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
    /// Every entity the targets name, in order.
    pub fn entities(&self) -> Vec<&EntityTarget> {
        match self {
            Self::Entity { entity, .. } => vec![entity],
            Self::Pair { endpoints, .. } => endpoints.iter().collect(),
            Self::Claim { .. } | Self::ClaimSet { .. } => vec![],
        }
    }
    fn version(&self) -> u32 {
        match self {
            Self::Entity { version, .. }
            | Self::Pair { version, .. }
            | Self::Claim { version, .. }
            | Self::ClaimSet { version, .. } => *version,
        }
    }
    fn endpoints(&self) -> Option<&[EntityTarget; 2]> {
        match self {
            Self::Pair { endpoints, .. } => Some(endpoints),
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
    fn empty() -> Self {
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
        }
    }
    /// Checks the row on its own: the envelope, the JSON shapes and the kind's target shape.
    /// What it says about other intents (an undo's target) is checked against the log.
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
            IntentKind::CannotLink | IntentKind::SameAs => {
                matches!(&self.targets, Targets::Pair { symmetric: true, endpoints, .. } if endpoints[0].id < endpoints[1].id)
            }
            IntentKind::Undo => !matches!(
                &self.targets,
                Targets::Claim { .. } | Targets::ClaimSet { .. }
            ),
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
                IntentKind::Undo => {}
            }
        }
        r
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
            let ended = match self.status.get(id) {
                Some((IntentStatus::Undone, Some(by))) => Some(self.issued[by]),
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
                _ => {}
            }
        }
        out
    }
    /// Each stretch during which a name intent gave its Person's name, as (Person id, name,
    /// intent id, (start, end)). Read in acceptance order: a Person's name is its newest name
    /// intent not yet undone, so a newer name ends the older one's stretch, and undoing the
    /// newer one starts a new stretch of the older at the undo, leaving a gap (plan D39).
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
                IntentKind::Undo => {
                    let Some(undone) = intent.undo_of.as_ref().and_then(|u| self.intents.get(u))
                    else {
                        continue;
                    };
                    let (IntentKind::Name, Some(person)) = (undone.kind, undone.targets.single())
                    else {
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
                && let Some(name) = self.intents[&after].payload.name.clone()
            {
                stretches.push((person, name, after, (at, None)));
            }
        }
        stretches
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
fn new_intent(
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
fn resolve(
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
    fn issue(
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

    #[test]
    fn S6_an_undo_removes_only_its_intents_contribution_and_a_link_clears_a_cannot_link() {
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

    #[test]
    fn S8_rows_are_checked_on_their_own() {
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

    /// S35: only an entity whose current types are all accounts may be held; an untyped one, a
    /// pull request, a repository or a bot may not, and the refusal names why
    #[test]
    fn S35_only_an_entity_typed_as_an_account_may_be_held() {
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

    #[test]
    fn S4_the_newest_name_wins_and_undoing_it_restores_the_older() {
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
