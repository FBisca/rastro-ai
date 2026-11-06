//! Protocol and manifest types shared by the core and its plugins (ADR 0003, ADR 0004,
//! ADR 0008).
//! Depends on nothing from the core.
use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
};

/// Protocol version this SDK speaks. Majors must match; minors may differ.
pub const PROTOCOL_VERSION: &str = "1.0";
pub const INITIALIZE: &str = "initialize";
pub const EVIDENCE_SUBMIT: &str = "evidence.submit";
pub const COVERAGE_REPORT: &str = "coverage.report";
pub const RUN_FINISH: &str = "run.finish";
pub const EVIDENCE_BATCH: &str = "evidence.batch";
pub const CLAIM_SUBMIT: &str = "claim.submit";

/// Why a connector couldn't look (decisions/knowledge.md §18). Anything else it reports is
/// recorded as `protocol`.
pub const COULDNT_LOOK_REASONS: [&str; 11] = [
    "auth",
    "forbidden",
    "not_found_or_hidden",
    "rate_limited",
    "network",
    "timeout",
    "protocol",
    "interrupted",
    "storage",
    "result_cap",
    "not_installed",
];

pub fn major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// One JSON-RPC 2.0 line. A request has `id` and `method`, a notification only `method`,
/// a response `id` and `result` or `error`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Envelope {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}
impl Envelope {
    pub fn request(id: u64, method: &str, params: impl Serialize) -> Result<Self> {
        Ok(Self {
            id: Some(id.into()),
            ..Self::notification(method, params)?
        })
    }
    pub fn notification(method: &str, params: impl Serialize) -> Result<Self> {
        Ok(Self {
            jsonrpc: "2.0".into(),
            method: Some(method.into()),
            params: Some(serde_json::to_value(params)?),
            ..Self::default()
        })
    }
    pub fn response(id: Option<Value>, result: impl Serialize) -> Result<Self> {
        Ok(Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(serde_json::to_value(result)?),
            ..Self::default()
        })
    }
    /// JSON-RPC 2.0 shape: the right version, an id that is a string or a number, and
    /// either a request/notification (`method`) or a response (`result` xor `error`).
    pub fn is_well_formed(&self) -> bool {
        let id_ok = self
            .id
            .as_ref()
            .is_none_or(|id| id.is_string() || id.is_number());
        let shape_ok = match &self.method {
            Some(_) => self.result.is_none() && self.error.is_none(),
            None => self.result.is_some() != self.error.is_some(),
        };
        self.jsonrpc == "2.0" && id_ok && shape_ok
    }
    pub fn params<T: for<'a> Deserialize<'a>>(&self) -> Result<T> {
        serde_json::from_value(self.params.clone().unwrap_or(Value::Null)).with_context(|| {
            format!(
                "invalid params for {}",
                self.method.as_deref().unwrap_or("?")
            )
        })
    }
    pub fn result<T: for<'a> Deserialize<'a>>(&self) -> Result<T> {
        serde_json::from_value(self.result.clone().context("response has no result")?)
            .context("invalid result")
    }
}

/// Half-open window `[from, to)` of RFC 3339 timestamps.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct Window {
    #[schemars(extend("format" = "date-time"))]
    pub from: String,
    #[schemars(extend("format" = "date-time"))]
    pub to: String,
}
/// What the core binds a run to. Evidence and coverage inherit all of it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunBinding {
    pub run_id: i64,
    pub source: String,
    pub scope: String,
    pub selector: BTreeMap<String, String>,
    pub window: Window,
    /// The source opted in to keeping the full original: send each observation's `original`.
    /// Without it the core drops any original.
    #[serde(default)]
    pub keep_original: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InitializeParams {
    pub protocol: String,
    pub manifest_id: String,
    pub run: RunBinding,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InitializeResult {
    pub protocol: String,
    pub manifest_id: String,
}

/// What a plugin is asked to do in a run. An `initialize` without one is a connector's.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Connector,
    Extractor,
}
/// One (source, scope) pair whose evidence an extractor run may read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceScope {
    pub source: String,
    pub scope: String,
}
/// What the core binds an extractor run to.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtractionBinding {
    pub run_id: i64,
    pub extractor_version: String,
    pub pairs: Vec<SourceScope>,
}
/// `initialize` for the extractor role.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtractorInitializeParams {
    pub protocol: String,
    pub manifest_id: String,
    pub role: Role,
    pub run: ExtractionBinding,
}
/// A field and a half-open UTF-8 byte range in it, with the quoted text.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Span {
    pub field: String,
    pub start: u64,
    pub end: u64,
    pub quote: String,
}
/// One stored observation sent to an extractor.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BatchItem {
    pub evidence_id: i64,
    pub source: String,
    pub scope: String,
    pub location: String,
    pub span: Span,
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_updated_at: Option<String>,
    pub trust_class: String,
}
/// `evidence.batch` (core to extractor), in (`captured_at`, evidence id) order. After the one
/// marked `last` the extractor sends `run.finish`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvidenceBatch {
    pub items: Vec<BatchItem>,
    pub last: bool,
}
/// An entity named by its canonical identifier, with its type.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EntityRef {
    pub identifier: String,
    #[serde(rename = "type")]
    pub entity_type: String,
}
/// What a claim says about its subject: another entity or a typed literal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClaimObject {
    Entity {
        identifier: String,
        #[serde(rename = "type")]
        entity_type: String,
    },
    Literal {
        value: String,
        datatype: String,
    },
}
/// Where in a delivered observation a claim is supported: its field and a byte range inside
/// that observation's span.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupportRef {
    pub evidence_id: i64,
    pub field: String,
    pub start: u64,
    pub end: u64,
}
/// `claim.submit` (extractor to core). The core validates every part (ADR 0008).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimSubmit {
    pub subject: EntityRef,
    pub predicate: String,
    pub object: ClaimObject,
    pub extraction_rule: String,
    pub supports: Vec<SupportRef>,
}

/// A field that is present, even as `null`, is `Some`; only an absent one is `None`.
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// `evidence.submit`. Every field is optional on the wire so the core, not the parser,
/// decides what to reject. The core sets the capture time.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EvidenceSubmit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_updated_at: Option<String>,
    /// Inherited from the binding; a different one is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    /// The item or comment as the connector read it. Sent only when the binding says
    /// `keep_original`; the core stores it beside the observation.
    /// An explicit `null` is kept as `Some(Value::Null)`, distinct from an omitted field.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub original: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Complete,
    CouldntLook,
}
/// A part of a read that couldn't be looked at, such as an unreadable `/user/emails`: the run stays found or ok.
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct CoverageGap {
    pub selection: String,
    pub reason: String,
}

/// `coverage.report`. Whether it was found or empty is the core's call.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoverageReport {
    pub status: CoverageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<CoverageGap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Inherited from the binding; a different one is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
}

/// `run.finish` request: the plugin's own view (items, or claims for an extractor). The core's counts are the authority.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunFinish {
    pub status: String,
    pub submitted: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Counts {
    pub submitted: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub unchanged: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Permissions {
    /// Evidence kinds the plugin may submit.
    pub submit: Vec<String>,
    /// Scopes the plugin may be bound to. A source whose scope isn't listed is refused;
    /// an empty list declares no scopes, so every source is refused.
    pub scopes: Vec<String>,
    pub actions: Vec<String>,
    /// Environment variables the plugin needs (for example `GH_TOKEN`). It starts with a
    /// cleared environment plus the basics (`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`).
    #[serde(default)]
    pub env: Vec<String>,
}
/// An extraction rule an extractor declares; only deterministic ones may yield observed claims.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtractionRule {
    pub id: String,
    pub deterministic: bool,
}
/// `rastro-plugin.toml`. A request: the source's grant can only narrow it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub kinds: Vec<String>,
    pub protocol: u32,
    pub command: String,
    pub permissions: Permissions,
    pub triggers: Vec<String>,
    /// Vocabularies (namespaces) an extractor uses.
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub extraction_rules: Vec<ExtractionRule>,
    /// Path, relative to the plugin directory, of the vocabulary this package provides.
    #[serde(default)]
    pub vocabulary: Option<String>,
    /// On a connector: the extractor bound by default to its sources.
    #[serde(default)]
    pub default_extractor: Option<String>,
    /// On a connector: the options a source of it takes (`source add --option key=value`).
    /// A manifest declaring none refuses any option.
    #[serde(default)]
    pub source_options: Vec<SourceOption>,
    /// On a connector: it takes one source at most, because what it reads doesn't depend on the
    /// source (one credential reads the same data whatever the source says), so a second source
    /// would read the same data into another scope. `source add` refuses a second one.
    #[serde(default)]
    pub single_source: bool,
}
/// One option a connector's sources take.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourceOption {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    /// A regular expression the whole value must match.
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub help: String,
}
impl SourceOption {
    /// The declared pattern, anchored over the whole value. The raw pattern must parse as one
    /// expression on its own, so it can't escape the anchors.
    fn matcher(&self) -> Result<regex::Regex> {
        let pattern = self.pattern.as_deref().unwrap_or(".*");
        regex::Regex::new(pattern)
            .and_then(|_| regex::Regex::new(&format!("^(?:{pattern})$")))
            .with_context(|| format!("option {} declares an invalid pattern", self.name))
    }
}
impl Manifest {
    /// Checks the options of a source against those declared. Refusals name the declared options.
    pub fn check_source_options(
        &self,
        options: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        Self::check_options(&self.id, &self.source_options, options)
    }
    /// Checks `options` against `declared` options of connector `connector`.
    pub fn check_options(
        connector: &str,
        declared_options: &[SourceOption],
        options: &std::collections::BTreeMap<String, String>,
    ) -> Result<()> {
        let declared = if declared_options.is_empty() {
            "none".to_string()
        } else {
            declared_options
                .iter()
                .map(|o| {
                    let required = if o.required { ", required" } else { "" };
                    format!("{} ({}{required})", o.name, o.help)
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        anyhow::ensure!(
            options.keys().all(|key| !key.is_empty()),
            "option names must not be empty; declared options: {declared}"
        );
        for key in options.keys() {
            anyhow::ensure!(
                declared_options.iter().any(|o| &o.name == key),
                "connector {connector} does not declare option {key}; declared options: {declared}"
            );
        }
        for option in declared_options {
            let Some(value) = options.get(&option.name) else {
                anyhow::ensure!(
                    !option.required,
                    "missing required option {}; declared options: {declared}",
                    option.name
                );
                continue;
            };
            if let Some(pattern) = &option.pattern {
                let regex = regex::Regex::new(&format!("^(?:{pattern})$")).with_context(|| {
                    format!("option {} declares an invalid pattern", option.name)
                })?;
                anyhow::ensure!(
                    regex.is_match(value),
                    "option {} must match the pattern {pattern} ({})",
                    option.name,
                    option.help
                );
            }
        }
        Ok(())
    }

    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(text).context("invalid rastro-plugin.toml")?;
        for option in &manifest.source_options {
            option.matcher()?;
        }
        Ok(manifest)
    }
}

/// A vocabulary file: one namespace's entity types, predicates and alias kinds, each defined
/// in words (decisions/vocabulary.md §4, §5, §10).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    pub namespace: String,
    pub version: String,
    pub types: Vec<TypeDefinition>,
    pub predicates: Vec<Predicate>,
    /// Kinds of alias its extractors may record, such as a login.
    #[serde(default)]
    pub aliases: Vec<AliasKind>,
}
/// What a term means, in the words extractors and reviewers judge claims against. Every field
/// is required and non-empty.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Definition {
    /// One sentence.
    pub definition: String,
    pub not_to_confuse_with: String,
    pub examples: Vec<String>,
    pub counter_examples: Vec<String>,
    /// The evidence a claim using the term needs.
    pub evidence_requirement: String,
}
/// An entity type.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TypeDefinition {
    pub id: String,
    /// A broader type of this vocabulary, or the core's `rastro:Account` ([`ACCOUNT_TYPE`]),
    /// which declares the type an account. A type and its supertype (at any distance) don't
    /// conflict when two submissions type one entity with them (decisions/vocabulary.md §15).
    #[serde(default)]
    pub supertype: Option<String>,
    #[serde(flatten)]
    pub about: Definition,
    /// Fields this format doesn't define; a file holding any is refused.
    #[serde(flatten, skip_serializing)]
    unknown: Unknown,
}
/// A predicate: the subject types it applies to, and the object types or literal datatypes
/// it takes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Predicate {
    pub id: String,
    pub domain: Vec<String>,
    pub range: Vec<String>,
    /// Required, with no default. Report-only: two live values for a scalar are a conflict,
    /// never a rejection (decisions/vocabulary.md §16).
    pub cardinality: Cardinality,
    #[serde(flatten)]
    pub about: Definition,
    #[serde(flatten, skip_serializing)]
    unknown: Unknown,
    pub freshness: Freshness,
    pub authority: Authority,
    /// Its default sensitivity, in words.
    pub sensitivity: String,
}
/// How many values a predicate takes for one subject and observation context.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    One,
    Many,
}
/// Fields a vocabulary entry doesn't define, kept only to refuse the file and name them.
/// (`deny_unknown_fields` can't be combined with the flattened definition.)
#[derive(Clone, Debug, Default, Deserialize)]
struct Unknown(std::collections::BTreeMap<String, toml::Value>);
/// How long an observation stays fresh.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// The source's own freshness rule.
    Source,
}
/// Which observation wins when sources disagree.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// The newer comparable observation (decisions/knowledge.md §26).
    NewerComparableObservation,
}
/// A kind of alias, and the entity types it names.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AliasKind {
    pub kind: String,
    pub domain: Vec<String>,
    #[serde(flatten)]
    pub about: Definition,
    #[serde(flatten, skip_serializing)]
    unknown: Unknown,
}
/// The prefix the core owns: its meta-predicates and datatypes live under it, so no
/// vocabulary may define a name there.
pub const CORE_PREFIX: &str = "rastro:";
/// The core's supertype of every account (decisions/vocabulary.md §27): a vocabulary declares
/// which of its types are accounts by naming it as their `supertype`. Only an account is held
/// by a Person.
pub const ACCOUNT_TYPE: &str = "rastro:Account";
/// The literal datatypes a predicate's range may name (lexical identity, vocabulary.md §14).
pub const DATATYPES: [&str; 5] = ["string", "integer", "number", "boolean", "datetime"];
impl Vocabulary {
    /// Parses a vocabulary file and refuses one with a term left undefined.
    pub fn parse(text: &str) -> Result<Self> {
        let vocabulary: Self = toml::from_str(text).context("invalid vocabulary")?;
        vocabulary.check()?;
        Ok(vocabulary)
    }
    pub fn has_type(&self, id: &str) -> bool {
        self.types.iter().any(|t| t.id == id)
    }
    /// Whether two types are one or each other's supertype, at any distance: two submissions
    /// typing an entity with them agree. Types it doesn't define are related only if equal.
    pub fn related(&self, a: &str, b: &str) -> bool {
        let above = |from: &str, target: &str| {
            let mut at = from;
            // A chain is at most as long as the file; `check` refuses cycles.
            for _ in 0..=self.types.len() {
                let Some(up) = self
                    .types
                    .iter()
                    .find(|t| t.id == at)
                    .and_then(|t| t.supertype.as_deref())
                else {
                    return false;
                };
                if up == target {
                    return true;
                }
                at = up;
            }
            false
        };
        a == b || above(a, b) || above(b, a)
    }
    /// The type's supertypes at every distance, sorted: what a claim typing an entity with it
    /// is compatible with. Empty for a type with no supertype or one this file doesn't define.
    pub fn ancestors(&self, id: &str) -> Vec<String> {
        let mut found = std::collections::BTreeSet::new();
        let mut at = id;
        // A chain is at most as long as the file; `check` refuses cycles.
        for _ in 0..=self.types.len() {
            let Some(up) = self
                .types
                .iter()
                .find(|t| t.id == at)
                .and_then(|t| t.supertype.as_deref())
            else {
                break;
            };
            found.insert(up.to_string());
            at = up;
        }
        found.into_iter().collect()
    }
    fn check(&self) -> Result<()> {
        let reserved = |name: &str| name.starts_with(CORE_PREFIX) || name == "rastro";
        anyhow::ensure!(
            !reserved(&self.namespace),
            "vocabulary namespace {:?} is under the core's `rastro:` prefix",
            self.namespace
        );
        let names = self.types.iter().map(|t| &t.id);
        let names = names.chain(self.predicates.iter().map(|p| &p.id));
        let names = names.chain(self.aliases.iter().map(|a| &a.kind));
        if let Some(name) = names.into_iter().find(|n| reserved(n)) {
            anyhow::bail!("vocabulary defines {name:?}, a name under the core's `rastro:` prefix");
        }
        // An alias kind sits between `:` and `/` in a canonical alias, so it can't be empty or
        // hold either or whitespace.
        if let Some(a) = self.aliases.iter().find(|a| {
            a.kind.is_empty() || a.kind.contains(['/', ':']) || a.kind.contains(char::is_whitespace)
        }) {
            anyhow::bail!(
                "alias kind {:?} is empty or contains `/`, `:` or whitespace",
                a.kind
            );
        }
        let unknown = self.types.iter().map(|t| (&t.id, &t.unknown));
        let unknown = unknown.chain(self.predicates.iter().map(|p| (&p.id, &p.unknown)));
        let unknown = unknown.chain(self.aliases.iter().map(|a| (&a.kind, &a.unknown)));
        for (id, Unknown(fields)) in unknown {
            if let Some(field) = fields.keys().next() {
                anyhow::bail!("vocabulary term {id:?} has an unknown field {field:?}");
            }
        }
        for t in &self.types {
            if let Some(up) = &t.supertype {
                // The core's account supertype is the one type outside the file it may name.
                anyhow::ensure!(
                    (self.has_type(up) || up == ACCOUNT_TYPE) && *up != t.id,
                    "type {:?} has supertype {up:?}, which is neither a type this vocabulary defines nor the core's {ACCOUNT_TYPE}",
                    t.id
                );
            }
            // A cycle never reaches a root: `related` is bounded, but a file with one is wrong.
            let mut at = &t.id;
            for _ in 0..=self.types.len() {
                match self
                    .types
                    .iter()
                    .find(|x| &x.id == at)
                    .and_then(|x| x.supertype.as_ref())
                {
                    Some(up) => at = up,
                    None => break,
                }
                anyhow::ensure!(*at != t.id, "type {:?} is its own supertype", t.id);
            }
        }
        let terms = self.types.iter().map(|t| (&t.id, &t.about, None));
        let terms = terms.chain(
            self.predicates
                .iter()
                .map(|p| (&p.id, &p.about, Some((&p.domain, &p.sensitivity)))),
        );
        let terms = terms.chain(self.aliases.iter().map(|a| (&a.kind, &a.about, None)));
        for (id, about, predicate) in terms {
            let blank = |text: &String| text.trim().is_empty();
            let none = |list: &Vec<String>| list.is_empty() || list.iter().any(blank);
            let missing = [
                ("id", blank(id)),
                ("definition", blank(&about.definition)),
                ("not_to_confuse_with", blank(&about.not_to_confuse_with)),
                ("examples", none(&about.examples)),
                ("counter_examples", none(&about.counter_examples)),
                ("evidence_requirement", blank(&about.evidence_requirement)),
                ("sensitivity", predicate.is_some_and(|(_, s)| blank(s))),
            ];
            if let Some((field, _)) = missing.iter().find(|(_, missing)| *missing) {
                anyhow::bail!("vocabulary term {id:?} has no {field}");
            }
        }
        let domains = self
            .predicates
            .iter()
            .map(|p| (&p.id, &p.domain, Some(&p.range)));
        let domains = domains.chain(self.aliases.iter().map(|a| (&a.kind, &a.domain, None)));
        for (id, domain, range) in domains {
            anyhow::ensure!(!domain.is_empty(), "vocabulary term {id:?} has no domain");
            anyhow::ensure!(
                range.is_none_or(|r| !r.is_empty()),
                "vocabulary term {id:?} has no range"
            );
            if let Some(t) = domain.iter().find(|t| !self.has_type(t)) {
                anyhow::bail!("vocabulary term {id:?} applies to {t:?}, which it doesn't define");
            }
            // A range entry is a type this file defines or a known datatype; a name with a
            // namespace that isn't a defined type (`rastro:` included) is refused.
            for entry in range.into_iter().flatten() {
                anyhow::ensure!(
                    self.has_type(entry) || DATATYPES.contains(&entry.as_str()),
                    "vocabulary term {id:?} takes {entry:?}, which is neither a type it defines nor a known datatype"
                );
            }
        }
        Ok(())
    }
}

/// NDJSON over a pair of streams: one JSON object per line.
pub struct Transport<R, W> {
    reader: R,
    writer: W,
}
impl<R: BufRead, W: Write> Transport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }
    /// `None` on end of input.
    pub fn read(&mut self) -> Result<Option<Envelope>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        serde_json::from_str(&line)
            .map(Some)
            .context("malformed line")
    }
    pub fn send(&mut self, message: &impl Serialize) -> Result<()> {
        self.send_line(&serde_json::to_string(message)?)
    }
    /// Sends text as-is, for connectors that need to misbehave on purpose.
    pub fn send_line(&mut self, line: &str) -> Result<()> {
        writeln!(self.writer, "{line}")?;
        self.writer.flush()?;
        Ok(())
    }
    /// Sends bytes as-is plus a newline, for connectors that need to send invalid UTF-8 on purpose.
    pub fn send_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
    pub fn into_writer(self) -> W {
        self.writer
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const WORDS: &str = r#"definition = "d"
not_to_confuse_with = "n"
examples = ["e"]
counter_examples = ["c"]
evidence_requirement = "r"
"#;
    fn vocabulary(predicate_extra: &str) -> String {
        format!(
            "namespace = \"x\"\nversion = \"1\"\n[[types]]\nid = \"x:T\"\n{WORDS}\
             [[predicates]]\nid = \"x:p\"\ndomain = [\"x:T\"]\nrange = [\"string\"]\n{WORDS}\
             {predicate_extra}"
        )
    }
    const POLICY: &str = "cardinality = \"one\"\nfreshness = \"source\"\nauthority = \"newer_comparable_observation\"\nsensitivity = \"s\"\n";
    fn error(text: &str) -> String {
        format!("{:#}", Vocabulary::parse(text).unwrap_err())
    }

    /// claims/R19: a vocabulary with every definition field loads
    #[test]
    fn a_vocabulary_with_every_definition_field_loads() {
        let parsed = Vocabulary::parse(&vocabulary(POLICY)).unwrap();
        assert!(parsed.has_type("x:T"));
        assert_eq!(parsed.predicates[0].freshness, Freshness::Source);
        assert_eq!(parsed.predicates[0].cardinality, Cardinality::One);
        assert_eq!(parsed.types[0].supertype, None);
        let many = POLICY.replace("\"one\"", "\"many\"");
        let parsed = Vocabulary::parse(&vocabulary(&many)).unwrap();
        assert_eq!(parsed.predicates[0].cardinality, Cardinality::Many);
    }

    /// claims/R20: a vocabulary missing a definition field is refused
    #[test]
    fn a_vocabulary_missing_a_definition_field_is_refused() {
        for (field, line) in [
            ("not_to_confuse_with", "not_to_confuse_with = \"n\"\n"),
            ("counter_examples", "counter_examples = [\"c\"]\n"),
        ] {
            let text = vocabulary(POLICY).replacen(line, "", 2);
            let error = error(&text);
            assert!(error.contains(field), "{error}");
        }
        let blank = vocabulary(POLICY).replacen("definition = \"d\"", "definition = \" \"", 1);
        assert!(Vocabulary::parse(&blank).is_err());
        let empty = vocabulary(POLICY).replacen("examples = [\"e\"]", "examples = []", 1);
        assert!(Vocabulary::parse(&empty).is_err());
        for policy in ["cardinality", "freshness", "authority", "sensitivity"] {
            let text: String = POLICY
                .lines()
                .filter(|l| !l.starts_with(policy))
                .map(|l| format!("{l}\n"))
                .collect();
            let error = error(&vocabulary(&text));
            assert!(error.contains(policy), "{policy}: {error}");
        }
        let unknown = POLICY.replace("\"source\"", "\"forever\"");
        assert!(Vocabulary::parse(&vocabulary(&unknown)).is_err());
        let foreign = vocabulary(POLICY).replace("domain = [\"x:T\"]", "domain = [\"y:U\"]");
        assert!(Vocabulary::parse(&foreign).is_err());
    }

    /// claims/R20: an unknown field or a bad cardinality is refused naming it
    #[test]
    fn an_unknown_field_or_a_bad_cardinality_is_refused_naming_it() {
        let top = format!("flavour = \"x\"\n{}", vocabulary(POLICY));
        assert!(error(&top).contains("flavour"), "{}", error(&top));
        for place in ["id = \"x:T\"\n", "id = \"x:p\"\n"] {
            let text = vocabulary(POLICY).replacen(place, &format!("{place}colour = \"red\"\n"), 1);
            assert!(error(&text).contains("colour"), "{}", error(&text));
        }
        let alias = format!(
            "{}[[aliases]]\nkind = \"k\"\ndomain = [\"x:T\"]\nshade = 1\n{WORDS}",
            vocabulary(POLICY)
        );
        assert!(error(&alias).contains("shade"), "{}", error(&alias));
        let bad = POLICY.replace("\"one\"", "\"several\"");
        assert!(Vocabulary::parse(&vocabulary(&bad)).is_err());
    }

    /// claims/R20: an alias kind with `/`, `:` or whitespace is refused
    #[test]
    fn an_alias_kind_with_a_separator_or_whitespace_is_refused() {
        for kind in ["", "a/b", "a:b", "a b", "a\tb"] {
            let alias = format!(
                "{}[[aliases]]\nkind = {kind:?}\ndomain = [\"x:T\"]\n{WORDS}",
                vocabulary(POLICY)
            );
            assert!(
                error(&alias).contains("alias kind"),
                "{kind}: {}",
                error(&alias)
            );
        }
    }

    /// claims/R20: a name under the rastro prefix is refused
    #[test]
    fn a_name_under_the_rastro_prefix_is_refused() {
        let named = |id: &str| vocabulary(POLICY).replacen("x:T", id, 2);
        assert!(
            error(&named("rastro:T")).contains("rastro:"),
            "{}",
            error(&named("rastro:T"))
        );
        let predicate = vocabulary(POLICY).replacen("x:p", "rastro:p", 1);
        assert!(
            error(&predicate).contains("rastro:p"),
            "{}",
            error(&predicate)
        );
        let alias = format!(
            "{}[[aliases]]\nkind = \"rastro:alias\"\ndomain = [\"x:T\"]\n{WORDS}",
            vocabulary(POLICY)
        );
        assert!(error(&alias).contains("rastro:alias"), "{}", error(&alias));
        let namespace =
            vocabulary(POLICY).replacen("namespace = \"x\"", "namespace = \"rastro\"", 1);
        assert!(
            error(&namespace).contains("rastro"),
            "{}",
            error(&namespace)
        );
    }

    /// claims/R5: a supertype relates two types so they do not conflict
    #[test]
    fn a_supertype_relates_two_types_so_they_do_not_conflict() {
        let text = format!(
            "namespace = \"x\"\nversion = \"1\"\npredicates = []\n\
             [[types]]\nid = \"x:Account\"\n{WORDS}\
             [[types]]\nid = \"x:Bot\"\nsupertype = \"x:Account\"\n{WORDS}\
             [[types]]\nid = \"x:Item\"\n{WORDS}"
        );
        let parsed = Vocabulary::parse(&text).unwrap();
        assert!(parsed.related("x:Bot", "x:Account") && parsed.related("x:Account", "x:Bot"));
        assert!(parsed.related("x:Item", "x:Item"));
        assert!(!parsed.related("x:Bot", "x:Item"));
        assert_eq!(parsed.ancestors("x:Bot"), ["x:Account"]);
        assert!(parsed.ancestors("x:Account").is_empty());
        // A supertype must be a type the file defines.
        let dangling = text.replace("supertype = \"x:Account\"", "supertype = \"x:Nothing\"");
        assert!(
            error(&dangling).contains("x:Nothing"),
            "{}",
            error(&dangling)
        );
    }

    /// identity/R32: a vocabulary declares which of its types are accounts by the core's supertype
    /// `rastro:Account`, the one core type it may name as a supertype
    #[test]
    fn a_vocabulary_declares_its_accounts_by_the_cores_account_supertype() {
        let text = format!(
            "namespace = \"x\"\nversion = \"1\"\npredicates = []\n\
             [[types]]\nid = \"x:Account\"\nsupertype = \"rastro:Account\"\n{WORDS}\
             [[types]]\nid = \"x:Admin\"\nsupertype = \"x:Account\"\n{WORDS}\
             [[types]]\nid = \"x:Bot\"\n{WORDS}"
        );
        let parsed = Vocabulary::parse(&text).unwrap();
        assert_eq!(parsed.ancestors("x:Account"), [ACCOUNT_TYPE]);
        assert_eq!(parsed.ancestors("x:Admin"), [ACCOUNT_TYPE, "x:Account"]);
        assert!(parsed.ancestors("x:Bot").is_empty());
        // Two account types are not each other's supertype: they still conflict on one entity.
        assert!(!parsed.related("x:Account", "x:Bot"));
        for other in ["rastro:Person", "rastro:Thing", "rastro:account"] {
            let named = text.replacen("rastro:Account", other, 1);
            assert!(error(&named).contains(other), "{other}: {}", error(&named));
        }
    }

    /// claims/R20: a supertype cycle is refused
    #[test]
    fn a_supertype_cycle_is_refused() {
        let text = format!(
            "namespace = \"x\"\nversion = \"1\"\npredicates = []\n\
             [[types]]\nid = \"x:A\"\nsupertype = \"x:B\"\n{WORDS}\
             [[types]]\nid = \"x:B\"\nsupertype = \"x:A\"\n{WORDS}"
        );
        assert!(error(&text).contains("supertype"), "{}", error(&text));
    }

    /// claims/R20: a range names a defined type or a known datatype
    #[test]
    fn a_range_names_a_defined_type_or_a_known_datatype() {
        for bad in ["x:Missing", "rastro:type", "text"] {
            let text = vocabulary(POLICY).replacen(
                "range = [\"string\"]",
                &format!("range = [\"{bad}\"]"),
                1,
            );
            assert!(error(&text).contains(bad), "{bad}: {}", error(&text));
        }
        let ok = vocabulary(POLICY).replacen(
            "range = [\"string\"]",
            "range = [\"x:T\", \"integer\"]",
            1,
        );
        assert!(Vocabulary::parse(&ok).is_ok());
    }
}
