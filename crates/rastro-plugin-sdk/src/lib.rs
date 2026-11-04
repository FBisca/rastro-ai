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
/// `coverage.report`. Whether it was found or empty is the core's call.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoverageReport {
    pub status: CoverageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
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
}
impl Manifest {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("invalid rastro-plugin.toml")
    }
}

/// A vocabulary file: one namespace's entity types and predicates.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Vocabulary {
    pub namespace: String,
    pub version: String,
    pub types: Vec<String>,
    pub predicates: Vec<Predicate>,
}
/// A predicate: the subject types it applies to, and the object types or literal datatypes
/// it takes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Predicate {
    pub id: String,
    pub domain: Vec<String>,
    pub range: Vec<String>,
}
impl Vocabulary {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("invalid vocabulary")
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
