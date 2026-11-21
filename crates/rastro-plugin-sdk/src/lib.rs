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

pub mod schema;

/// Protocol version this SDK speaks. Majors must match; minors may differ.
pub const PROTOCOL_VERSION: &str = "1.0";
pub const INITIALIZE: &str = "initialize";
pub const EVIDENCE_SUBMIT: &str = "evidence.submit";
pub const COVERAGE_REPORT: &str = "coverage.report";
pub const RUN_FINISH: &str = "run.finish";
pub const EVIDENCE_BATCH: &str = "evidence.batch";
pub const CLAIM_SUBMIT: &str = "claim.submit";
/// `judgment.request` (core to agent tool adapter): content to judge, answered with candidates.
pub const JUDGMENT_REQUEST: &str = "judgment.request";
/// `decision.request` (core to decision adapter): the caller's instructions, input and output
/// schema, answered with structured output the core validates against that schema (ADR-0016 §5).
pub const DECISION_REQUEST: &str = "decision.request";
/// The kind of plugin that lets an agent runtime do judgment (decisions/plugins.md §2, §10).
pub const AGENT_TOOL_ADAPTER_KIND: &str = "agent-tool-adapter";
/// The kind of plugin that executes actions (ADR 0015). It reads no evidence: it declares its
/// actions and holds no `submit` permission.
pub const ACTION_PROVIDER_KIND: &str = "action_provider";
/// `action.execute` (core to action provider): do the action once, keyed by the run's id.
pub const ACTION_EXECUTE: &str = "action.execute";
/// `action.lookup` (core to action provider): find what a run's id already did, without doing it.
pub const ACTION_LOOKUP: &str = "action.lookup";
/// `action.undo` (core to action provider): reverse what a run did, when the action can be undone.
pub const ACTION_UNDO: &str = "action.undo";
/// What an agent tool adapter may ask of an agent tool: abilities, never a named tool.
/// `judge_text` is an agent adapter's judgment; `score_text` a decision adapter's scoring.
pub const AGENT_TOOL_CAPABILITIES: [&str; 2] = ["judge_text", "score_text"];
/// The boundary controls of the judgment floor (ADR-0016 §6): tools off, no MCP servers, no
/// setting sources. An adapter declares in its manifest which of these it can enforce; the core
/// matches that declaration against the floor before a run and refuses one that cannot enforce
/// a control the floor requires, naming it. The floor is the core's, never task-declarable.
pub const JUDGMENT_FLOOR: [&str; 3] = ["tools_off", "no_mcp_servers", "no_setting_sources"];

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
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
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
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct InitializeParams {
    pub protocol: String,
    pub manifest_id: String,
    pub run: RunBinding,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct InitializeResult {
    pub protocol: String,
    pub manifest_id: String,
}

/// Who an action affects (GLOSSARY "Reach").
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    OnlyMe,
    MyThings,
    OthersSeeIt,
    Public,
}
impl Reach {
    /// The manifest's names, in order.
    pub const NAMES: [&'static str; 4] = ["only_me", "my_things", "others_see_it", "public"];
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "only_me" => Some(Self::OnlyMe),
            "my_things" => Some(Self::MyThings),
            "others_see_it" => Some(Self::OthersSeeIt),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
    /// The words the owner reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::OnlyMe => "only me",
            Self::MyThings => "my things",
            Self::OthersSeeIt => "others see it",
            Self::Public => "public",
        }
    }
}
/// Whether an action can be undone (GLOSSARY "Reversibility").
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    CanBeUndone,
    CanBeCompensated,
    CannotBeUndone,
}
impl Reversibility {
    /// The manifest's names, in order.
    pub const NAMES: [&'static str; 3] =
        ["can_be_undone", "can_be_compensated", "cannot_be_undone"];
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "can_be_undone" => Some(Self::CanBeUndone),
            "can_be_compensated" => Some(Self::CanBeCompensated),
            "cannot_be_undone" => Some(Self::CannotBeUndone),
            _ => None,
        }
    }
    /// The words the owner reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::CanBeUndone => "can be undone",
            Self::CanBeCompensated => "can be compensated",
            Self::CannotBeUndone => "cannot be undone",
        }
    }
}

/// What a plugin is asked to do in a run. An `initialize` without one is a connector's.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Connector,
    Extractor,
    AgentToolAdapter,
    ActionProvider,
}
/// `initialize` for the action provider role. The provider holds no run binding: each action
/// message carries what it needs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActionInitializeParams {
    pub protocol: String,
    pub manifest_id: String,
    pub role: Role,
}
/// The params of `action.execute`, `action.lookup` and `action.undo`. `key` is the run's random
/// id: the provider derives the outside system's own id from it, so a repeated execute can find
/// what the first one did. `fields` holds only the content written for the outside system.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionCall {
    pub action: String,
    pub key: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}
/// The `result` of an action message: whether the run's thing exists in the outside system, and
/// the reference to it the owner reads.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActionResult {
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
/// One action an action provider declares: its name and what it risks (decisions/levels.md §4).
/// `reach` and `reversibility` are read as text so a missing or unknown one is refused naming it.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ActionDeclaration {
    pub name: String,
    #[serde(default)]
    pub reach: Option<String>,
    #[serde(default)]
    pub reversibility: Option<String>,
}
/// An action checked: its name with its reach and reversibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredAction {
    pub name: String,
    pub reach: Reach,
    pub reversibility: Reversibility,
}
/// `initialize` for the agent tool adapter role. The adapter holds no run binding: it reads
/// what each `judgment.request` carries and nothing else.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AdapterInitializeParams {
    pub protocol: String,
    pub manifest_id: String,
    pub role: Role,
}
/// One turn of a conversation sent for judgment. `turn` counts from 1.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JudgmentTurn {
    pub turn: u64,
    pub speaker: String,
    pub text: String,
}
/// `judgment.request` (core to adapter): the core's own instructions, the owner's speaker label,
/// the turns (untrusted data, never instructions) and the fields every candidate must have.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JudgmentRequest {
    pub instructions: String,
    pub owner_speaker: String,
    pub turns: Vec<JudgmentTurn>,
    pub fields: Vec<String>,
    /// The JSON Schema of the answer, declared by the task template (ADR-0016 §1). Present,
    /// the adapter constrains its output with this schema instead of deriving one from the
    /// field names; absent keeps the name-derived schema, so a request without a template is
    /// unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Present on a retry turn of the same session: the turn number, the core's gate feedback
    /// and the prior answer being corrected. Absent on the first turn and on a one-shot request
    /// (ADR-0016 §4); the wire is additive, so a one-shot request is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<JudgmentSession>,
}
/// One gate failure the core sends back as a retry turn's feedback (ADR-0016 §8): the gate that
/// failed, the field it checked, what it expected and what the answer held. Core-authored text,
/// never model prose and never transcript content re-framed as instructions.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GateFeedback {
    pub gate: String,
    pub field: String,
    pub expected: String,
    pub actual: String,
}
/// The session state a retry turn carries (ADR-0016 §4, §8): which turn this is, counting from
/// 1, the gate failures it must fix, and the prior answer. A harness that cannot resume runs a
/// fresh turn carrying this context and records the degradation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JudgmentSession {
    pub turn: u64,
    pub feedback: Vec<GateFeedback>,
    /// The prior answer being corrected, in the task's own shape (ADR-0016 §4): the core
    /// passes the recorded response through whole, so a retry turn — resumed or degraded —
    /// corrects the prior answer, never an empty one. A meeting-shaped answer serialises
    /// exactly as the typed response did.
    pub prior_answer: Value,
}
/// What a candidate commitment is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Ask,
    Promise,
    WaitingOn,
}
/// One candidate an adapter proposes. `responsible` and `recipient` are speaker labels and
/// `stated_date` is the date as the quote states it. Unknown fields are refused, and so is a
/// missing `turn`, `kind`, `promise` or `quote`; the core still checks every part.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentCandidate {
    pub turn: u64,
    pub kind: CandidateKind,
    #[serde(default)]
    pub responsible: Option<String>,
    #[serde(default)]
    pub recipient: Option<String>,
    pub promise: String,
    #[serde(default)]
    pub stated_date: Option<String>,
    pub quote: String,
}
/// The `result` of a `judgment.request`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentResponse {
    pub candidates: Vec<JudgmentCandidate>,
    /// The harness session id this answer produced, when the adapter's harness keeps one; a
    /// retry turn resumes it in a fresh process (ADR-0016 §4). Absent when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `true` when the adapter could not resume and ran a fresh turn carrying the prior answer
    /// and the failure reason; the degradation is recorded, never silent (ADR-0016 §4). Absent
    /// means the turn was not degraded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<bool>,
}
/// `decision.request` (core to decision adapter): the caller's own instructions, the input to
/// score and the schema the output must pass. The input and schema are caller-defined JSON; the
/// core validates the answer against the schema, the adapter never its own trust boundary
/// (ADR-0016 §2, §5).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DecisionRequest {
    pub instructions: String,
    pub input: Value,
    pub schema: Value,
}
/// The `result` of a `decision.request`: the adapter's structured output, which the core keeps
/// only if it passes the caller's schema.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionResponse {
    pub output: Value,
}
/// The type of one declared candidate field: a generic primitive, never a task's domain type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFieldType {
    String,
    Integer,
}
/// One field of a task template's output shape. `values` enumerates the only strings the field
/// may hold; empty declares any string. An absent field is required unless it says otherwise.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: TaskFieldType,
    #[serde(default = "required")]
    pub required: bool,
    #[serde(default)]
    pub values: Vec<String>,
}
fn required() -> bool {
    true
}
/// What a rule does with a candidate it matches (ADR-0016 §2, §8).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleDisposition {
    /// A gate failure: the answer earns a retry turn carrying the core's feedback.
    #[default]
    Fail,
    /// The candidate is left out and counted as excluded; never a retry.
    Exclude,
}
/// A source of text for a candidate predicate. Absent fields and turns yield no text.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskText {
    Field {
        name: String,
    },
    InputLabel,
    TurnLabel {
        turn_field: String,
        at: RelativeTurn,
    },
}

/// The named turn or one of its immediate neighbors. Replay needs only their labels.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeTurn {
    Previous,
    Current,
    Next,
}

/// A deterministic predicate over candidate fields and task input. Missing text never
/// satisfies a comparison. Label matching ignores case and respects whole-word boundaries.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "test", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskPredicate {
    All { of: Vec<Self> },
    Any { of: Vec<Self> },
    Not { of: Box<Self> },
    Equal { left: TaskText, right: TaskText },
    ContainsLabel { field: String, label: TaskText },
    ContainsWord { field: String, words: Vec<String> },
    InInputLabels { field: String },
}
impl TaskPredicate {
    fn check(&self, template: &TaskTemplate) -> Result<()> {
        let check_text = |text: &TaskText| match text {
            TaskText::Field { name } => {
                template.rule_field(name, TaskFieldType::String).map(|_| ())
            }
            TaskText::TurnLabel { turn_field, .. } => template
                .rule_field(turn_field, TaskFieldType::Integer)
                .map(|_| ()),
            TaskText::InputLabel => Ok(()),
        };
        match self {
            Self::All { of } | Self::Any { of } => {
                anyhow::ensure!(!of.is_empty(), "a predicate group cannot be empty");
                for predicate in of {
                    predicate.check(template)?;
                }
            }
            Self::Not { of } => of.check(template)?,
            Self::Equal { left, right } => {
                check_text(left)?;
                check_text(right)?;
            }
            Self::ContainsLabel { field, label } => {
                template.rule_field(field, TaskFieldType::String)?;
                check_text(label)?;
            }
            Self::ContainsWord { field, .. } | Self::InInputLabels { field } => {
                template.rule_field(field, TaskFieldType::String)?;
            }
        }
        Ok(())
    }
}

/// One declared rule over the candidates. The core executes it; the adapter never validates
/// its own trust boundary.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskRule {
    /// The quote field must be an exact, contiguous, non-empty piece of the text of the turn
    /// the turn field names.
    Span {
        quote_field: String,
        turn_field: String,
        #[serde(default)]
        disposition: RuleDisposition,
    },
    /// The field holds exactly this value.
    Equals {
        field: String,
        value: String,
        #[serde(default)]
        disposition: RuleDisposition,
    },
    /// Exclude a candidate unless its fields and task input satisfy the predicate.
    Require { predicate: TaskPredicate },
    /// Clear a text field unless it is a non-blank literal substring of another
    /// field. This changes the validated result, not the recorded candidate.
    Ground { field: String, in_field: String },
}
/// A task template: the declarative package a generic runner executes (ADR-0016 §1–2, §6;
/// decisions/plugins.md §40). The instructions and field names go to the adapter as the
/// request's own; the rules are the core's gates, each with its disposition; the boundary
/// names controls the adapter must enforce. The runner holds no task's domain logic and no
/// template identity: the same declarations run a meeting, a blog or any other task.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskTemplate {
    pub instructions: String,
    pub fields: Vec<TaskField>,
    #[serde(default)]
    pub rules: Vec<TaskRule>,
    /// Controls the task requires beyond the judgment floor (ADR-0016 §6). A name from
    /// [`JUDGMENT_FLOOR`] repeats the floor; any other name is an additional control the
    /// adapter must declare it can enforce. The floor itself is the core's, never
    /// task-declarable: a template can demand more lockdown, never less.
    #[serde(default)]
    pub boundary: Vec<String>,
}
impl TaskTemplate {
    /// Parses a declarative template and refuses one the runner cannot execute.
    pub fn parse(text: &str) -> Result<Self> {
        let template: Self = toml::from_str(text).context("invalid task template")?;
        template.check()?;
        Ok(template)
    }
    /// Checks the declarations name what the runner can execute: fields the rules reference,
    /// primitives of the right type, and a non-empty instruction and boundary names.
    pub fn check(&self) -> Result<()> {
        anyhow::ensure!(
            !self.instructions.trim().is_empty(),
            "the template declares no instructions"
        );
        anyhow::ensure!(!self.fields.is_empty(), "the template declares no fields");
        for field in &self.fields {
            anyhow::ensure!(
                !field.name.trim().is_empty(),
                "the template declares a field with no name"
            );
            anyhow::ensure!(
                self.fields.iter().filter(|f| f.name == field.name).count() == 1,
                "the template declares the field {} twice",
                field.name
            );
            anyhow::ensure!(
                field.field_type != TaskFieldType::Integer || field.values.is_empty(),
                "the field {} is an integer and declares enum values",
                field.name
            );
            anyhow::ensure!(
                field.values.iter().all(|value| !value.is_empty()),
                "the field {} declares an empty enum value",
                field.name
            );
        }
        for rule in &self.rules {
            match rule {
                TaskRule::Span {
                    quote_field,
                    turn_field,
                    ..
                } => {
                    let quote = self.field(quote_field).with_context(|| {
                        format!("the span rule names the undeclared field {quote_field}")
                    })?;
                    let turn = self.field(turn_field).with_context(|| {
                        format!("the span rule names the undeclared field {turn_field}")
                    })?;
                    anyhow::ensure!(
                        quote.field_type == TaskFieldType::String,
                        "the span rule's quote field {quote_field} is not a string"
                    );
                    anyhow::ensure!(
                        turn.field_type == TaskFieldType::Integer,
                        "the span rule's turn field {turn_field} is not an integer"
                    );
                }
                TaskRule::Equals { field, .. } => {
                    let declared = self.field(field).with_context(|| {
                        format!("the equals rule names the undeclared field {field}")
                    })?;
                    anyhow::ensure!(
                        declared.field_type == TaskFieldType::String,
                        "the equals rule's field {field} is not a string"
                    );
                }
                TaskRule::Require { predicate } => predicate.check(self)?,
                TaskRule::Ground { field, in_field } => {
                    self.rule_field(field, TaskFieldType::String)?;
                    self.rule_field(in_field, TaskFieldType::String)?;
                }
            }
        }
        for control in &self.boundary {
            anyhow::ensure!(
                !control.trim().is_empty(),
                "the boundary declares an empty control name"
            );
        }
        Ok(())
    }
    fn rule_field(&self, name: &str, kind: TaskFieldType) -> Result<&TaskField> {
        let field = self
            .field(name)
            .with_context(|| format!("a rule names the undeclared field {name}"))?;
        anyhow::ensure!(
            field.field_type == kind,
            "a rule needs field {name} to have type {kind:?}"
        );
        Ok(field)
    }
    /// The declared field named `name`, if the template declares one.
    pub fn field(&self, name: &str) -> Option<&TaskField> {
        self.fields.iter().find(|field| field.name == name)
    }
    /// The JSON Schema of the answer this template declares: candidates with exactly its
    /// fields, each the type and values declared, the required ones required (ADR-0016 §1).
    /// The envelope is the protocol's; the adapter adds the session fields itself.
    pub fn output_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for field in &self.fields {
            let kind = match field.field_type {
                TaskFieldType::Integer => "integer",
                TaskFieldType::String => "string",
            };
            let mut property = if field.required {
                serde_json::json!({ "type": kind })
            } else {
                serde_json::json!({ "type": [kind, "null"] })
            };
            if !field.values.is_empty() {
                let mut values: Vec<Value> = field
                    .values
                    .iter()
                    .map(|value| Value::String(value.clone()))
                    .collect();
                if !field.required {
                    values.push(Value::Null);
                }
                property["enum"] = Value::Array(values);
            }
            properties.insert(field.name.clone(), property);
            if field.required {
                required.push(field.name.clone());
            }
        }
        serde_json::json!({
            "type": "object",
            "properties": {"candidates": {"type": "array", "items": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            }}},
            "required": ["candidates"],
            "additionalProperties": false,
        })
    }
}

/// What an agent tool adapter needs from an agent tool, in capabilities. It never names a tool
/// (decisions/plugins.md §4): fields this declaration doesn't define are kept only to refuse it.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct AgentToolNeeds {
    pub capabilities: Vec<String>,
    #[serde(flatten, skip_serializing)]
    #[schemars(skip)]
    unknown: std::collections::BTreeMap<String, toml::Value>,
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
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
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
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
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
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ExtractionRule {
    pub id: String,
    pub deterministic: bool,
}
/// `rastro-plugin.toml`. A request: the source's grant can only narrow it.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub kinds: Vec<String>,
    pub protocol: u32,
    pub command: String,
    pub permissions: Permissions,
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Vocabularies (namespaces) an extractor uses.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// On an agent tool adapter: the capabilities it needs from an agent tool, never a named tool.
    #[serde(default)]
    pub agent_tool: Option<AgentToolNeeds>,
    /// On an agent tool adapter: the boundary controls it can enforce — the judgment floor's
    /// ([`JUDGMENT_FLOOR`]) and any additional control a task's boundary demands (ADR-0016 §6).
    /// Absent declares none; the core refuses a run whose floor or task boundary the declaration
    /// cannot match, naming the unenforceable control, and never starts the adapter in a
    /// degraded-security mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enforcement: Vec<String>,
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
    /// On an action provider: the actions it declares, each with its reach and reversibility
    /// (`[[action]]`). Any other plugin that declares one is refused.
    #[serde(default, rename = "action")]
    pub actions: Vec<ActionDeclaration>,
}
/// One option a connector's sources take.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
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

    /// Checks an agent tool adapter's declaration: it declares the kind, and the agent-tool
    /// capabilities it needs from [`AGENT_TOOL_CAPABILITIES`]. A tool named anywhere in the
    /// declaration (an unknown field, or a capability that is not one) is refused.
    pub fn check_agent_tool(&self) -> Result<()> {
        anyhow::ensure!(
            self.kinds.iter().any(|k| k == AGENT_TOOL_ADAPTER_KIND),
            "plugin {} does not declare the kind {AGENT_TOOL_ADAPTER_KIND}",
            self.id
        );
        let needs = self
            .agent_tool
            .as_ref()
            .with_context(|| format!("plugin {} declares no `agent_tool` capabilities", self.id))?;
        if let Some(field) = needs.unknown.keys().next() {
            anyhow::bail!(
                "plugin {} names {field:?} in `agent_tool`: declare capabilities, never a named tool",
                self.id
            );
        }
        anyhow::ensure!(
            !needs.capabilities.is_empty(),
            "plugin {} declares no agent-tool capability",
            self.id
        );
        if let Some(capability) = needs
            .capabilities
            .iter()
            .find(|c| !AGENT_TOOL_CAPABILITIES.contains(&c.as_str()))
        {
            anyhow::bail!(
                "plugin {} needs {capability:?}, which is not an agent-tool capability ({}): declare capabilities, never a named tool",
                self.id,
                AGENT_TOOL_CAPABILITIES.join(", ")
            );
        }
        Ok(())
    }

    /// The controls of the judgment floor plus the task's `boundary` that this manifest does
    /// not declare it can enforce, floor first, without duplicates (ADR-0016 §6). The floor
    /// is always required; a boundary can demand more lockdown, never less.
    pub fn unenforceable(&self, boundary: &[String]) -> Vec<String> {
        let mut required: Vec<&str> = JUDGMENT_FLOOR.to_vec();
        for control in boundary {
            if !required.contains(&control.as_str()) {
                required.push(control.as_str());
            }
        }
        required
            .into_iter()
            .filter(|control| !self.enforcement.iter().any(|declared| declared == control))
            .map(str::to_string)
            .collect()
    }

    /// Checks an action provider's declaration and returns its actions. A provider is that kind
    /// and no other, declares at least one action with its reach and reversibility, lists the
    /// same names in `permissions.actions`, and holds no `submit` permission: it never reads
    /// evidence, so untrusted content never reaches a plugin that can write (ADR 0015).
    pub fn check_action_provider(&self) -> Result<Vec<DeclaredAction>> {
        let id = &self.id;
        if let Some(kind) = self.kinds.iter().find(|k| *k != ACTION_PROVIDER_KIND) {
            anyhow::bail!(
                "plugin {id} is also a {kind}: an action provider is that kind and no other"
            );
        }
        anyhow::ensure!(
            self.kinds.iter().any(|k| k == ACTION_PROVIDER_KIND),
            "plugin {id} does not declare the kind {ACTION_PROVIDER_KIND}"
        );
        anyhow::ensure!(
            self.permissions.submit.is_empty(),
            "plugin {id} is an action provider and holds the `submit` permission: it declares actions or reads evidence, never both"
        );
        anyhow::ensure!(!self.actions.is_empty(), "plugin {id} declares no action");
        let mut declared = Vec::new();
        for action in &self.actions {
            let name = &action.name;
            let reach = Self::risk(id, name, "reach", action.reach.as_deref(), &Reach::NAMES)?;
            let reversibility = Self::risk(
                id,
                name,
                "reversibility",
                action.reversibility.as_deref(),
                &Reversibility::NAMES,
            )?;
            anyhow::ensure!(
                !declared.iter().any(|d: &DeclaredAction| &d.name == name),
                "plugin {id} declares the action {name} twice"
            );
            declared.push(DeclaredAction {
                name: name.clone(),
                reach: Reach::parse(reach).context("reach was checked")?,
                reversibility: Reversibility::parse(reversibility)
                    .context("reversibility was checked")?,
            });
        }
        let mut names: Vec<_> = declared.iter().map(|d| d.name.as_str()).collect();
        let mut granted: Vec<_> = self
            .permissions
            .actions
            .iter()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        granted.sort_unstable();
        anyhow::ensure!(
            names == granted,
            "plugin {id} declares the actions {} but lists {} in `permissions.actions`: they must be the same",
            names.join(", "),
            granted.join(", ")
        );
        Ok(declared)
    }
    /// One of an action's risks: present and one of the `known` names.
    fn risk<'a>(
        id: &str,
        action: &str,
        field: &str,
        value: Option<&'a str>,
        known: &[&str],
    ) -> Result<&'a str> {
        let value = value.with_context(|| {
            format!("plugin {id} declares the action {action} with no `{field}`")
        })?;
        anyhow::ensure!(
            known.contains(&value),
            "plugin {id} declares the action {action} with the `{field}` {value:?}, which is none of {}",
            known.join(", ")
        );
        Ok(value)
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
    use serde_json::json;

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
    /// spec: The GitHub extractor emits cited code claims
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
    /// spec: The core refuses a vocabulary it cannot hold
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
    /// spec: The core refuses a vocabulary it cannot hold
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
    /// spec: Alias kinds and values are checked
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
    /// spec: The core refuses a vocabulary it cannot hold
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

    /// spec: Only account types may be linked
    /// A vocabulary declares which of its types are accounts by the core's supertype
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

    /// An adapter manifest declaring one capability and the judgment floor's boundary controls,
    /// with no named tool.
    fn adapter_manifest(capability: &str) -> String {
        format!(
            "id = \"rastro-adapter-test\"\nname = \"Test adapter\"\nversion = \"0.1.0\"\n\
             author = \"Rastro\"\nkinds = [\"agent-tool-adapter\"]\nprotocol = 1\n\
             command = \"rastro-adapter-test\"\n\
             agent_tool = {{ capabilities = [\"{capability}\"] }}\n\
             enforcement = [\"tools_off\", \"no_mcp_servers\", \"no_setting_sources\"]\n\
             [permissions]\nsubmit = []\nscopes = []\nactions = []\n"
        )
    }

    /// S9: a codex, pi or decision adapter's manifest passes the check as kind
    /// agent-tool-adapter with its declared capability and no named tool.
    // spec: An adapter's manifest declares its kind and names no tool
    #[test]
    fn a_harness_or_decision_adapter_manifest_passes_the_check() {
        // codex and pi are harness adapters needing judge_text; a decision adapter needs
        // score_text.
        for capability in ["judge_text", "score_text"] {
            let manifest = Manifest::parse(&adapter_manifest(capability)).unwrap();
            manifest
                .check_agent_tool()
                .unwrap_or_else(|e| panic!("{capability}: {e:#}"));
            assert_eq!(manifest.kinds, [AGENT_TOOL_ADAPTER_KIND]);
            assert_eq!(
                manifest.agent_tool.unwrap().capabilities,
                [capability.to_string()]
            );
        }
    }

    /// A manifest keeps the boundary enforcement it declares, and one without the field still
    /// parses: the declaration the core matches is exactly what the adapter wrote.
    // spec: An adapter's manifest declares its kind and names no tool
    #[test]
    fn a_manifest_keeps_its_declared_boundary_enforcement() {
        let manifest = Manifest::parse(&adapter_manifest("judge_text")).unwrap();
        let wire = serde_json::to_value(&manifest).unwrap();
        assert_eq!(
            wire["enforcement"],
            json!(["tools_off", "no_mcp_servers", "no_setting_sources"])
        );
        let bare = adapter_manifest("judge_text").replace(
            "enforcement = [\"tools_off\", \"no_mcp_servers\", \"no_setting_sources\"]\n",
            "",
        );
        let manifest = Manifest::parse(&bare).unwrap();
        let wire = serde_json::to_value(&manifest).unwrap();
        assert!(wire.get("enforcement").is_none(), "{wire}");
    }

    /// The judgment wire is additive: a one-shot request and answer carry no session fields,
    /// and a retry turn's session object and the answer's session id and degradation survive
    /// the wire unchanged.
    // spec: The core calls the adapter and keeps only the candidates that pass its checks
    #[test]
    fn the_judgment_session_wire_is_additive() {
        let one_shot = json!({
            "instructions": "Find commitments.",
            "owner_speaker": "Alice",
            "turns": [{"turn": 1, "speaker": "Bob", "text": "I will send it."}],
            "fields": ["turn", "kind", "quote"]
        });
        let request: JudgmentRequest = serde_json::from_value(one_shot.clone()).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), one_shot);

        let retry = json!({
            "instructions": "Find commitments.",
            "owner_speaker": "Alice",
            "turns": [{"turn": 1, "speaker": "Bob", "text": "I will send it."}],
            "fields": ["turn", "kind", "quote"],
            "session": {
                "turn": 2,
                "feedback": [{
                    "gate": "quote",
                    "field": "quote",
                    "expected": "an exact, contiguous piece of turn 1",
                    "actual": "I will send it tomorrow"
                }],
                "prior_answer": {"candidates": []}
            }
        });
        let request: JudgmentRequest = serde_json::from_value(retry.clone()).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), retry);

        // A prior answer of another task's shape survives the wire whole, and a declared
        // output schema rides along: the session carries the recorded response, not a
        // meeting-shaped parse of it.
        let blog_retry = json!({
            "instructions": "Propose blog summaries.",
            "owner_speaker": "",
            "turns": [{"turn": 1, "speaker": "post", "text": "Rastro keeps every quote exact."}],
            "fields": ["section", "title", "excerpt", "status"],
            "output_schema": {"type": "object"},
            "session": {
                "turn": 2,
                "feedback": [{
                    "gate": "span",
                    "field": "candidates[0].excerpt",
                    "expected": "an exact, contiguous piece of turn 1's text",
                    "actual": "words the section never says"
                }],
                "prior_answer": {
                    "candidates": [{
                        "section": 1,
                        "title": "Exact quotes",
                        "excerpt": "words the section never says",
                        "status": "published"
                    }],
                    "session_id": "sess-1"
                }
            }
        });
        let request: JudgmentRequest = serde_json::from_value(blog_retry.clone()).unwrap();
        assert_eq!(serde_json::to_value(&request).unwrap(), blog_retry);

        let one_shot_answer = json!({"candidates": []});
        let answer: JudgmentResponse = serde_json::from_value(one_shot_answer.clone()).unwrap();
        assert_eq!(serde_json::to_value(&answer).unwrap(), one_shot_answer);

        let session_answer = json!({"candidates": [], "session_id": "s-1", "degraded": true});
        let answer: JudgmentResponse = serde_json::from_value(session_answer.clone()).unwrap();
        assert_eq!(serde_json::to_value(&answer).unwrap(), session_answer);
    }

    /// A decision request carries the caller's instructions, input and schema, and its answer
    /// carries the structured output the core validates against that schema.
    #[test]
    fn a_decision_request_and_response_round_trip() {
        let request = json!({
            "instructions": "Score the text.",
            "input": {"text": "I will send it."},
            "schema": {"type": "object", "properties": {"score": {"type": "number"}}}
        });
        let message = Envelope::request(1, DECISION_REQUEST, &request).unwrap();
        let parsed: DecisionRequest = message.params().unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), request);

        let output = json!({"output": {"score": 0.9}});
        let answer = Envelope::response(Some(1.into()), &output).unwrap();
        let parsed: DecisionResponse = answer.result().unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), output);
    }

    /// spec: Task templates run without domain logic in the core runner
    /// The floor and the task boundary are matched against the declaration; every
    /// unenforceable control is named once, floor first.
    #[test]
    fn missing_controls_are_named() {
        fn manifest(enforcement: &str) -> Manifest {
            Manifest::parse(&format!(
                r#"
id = "m"
name = "M"
version = "0.1.0"
author = "Rastro"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "m"
{enforcement}
[permissions]
submit = []
scopes = []
env = []
actions = []
"#
            ))
            .unwrap()
        }
        let full = "enforcement = [\"tools_off\", \"no_mcp_servers\", \"no_setting_sources\"]";
        let partial = "enforcement = [\"tools_off\", \"no_mcp_servers\"]";
        let extra = "enforcement = [\"tools_off\", \"no_mcp_servers\", \"no_setting_sources\", \"no_session_persistence\"]";
        let boundary = vec!["no_session_persistence".to_string()];
        for (name, declaration, profile, expected) in [
            (
                "no declaration at all",
                "",
                vec![],
                vec!["tools_off", "no_mcp_servers", "no_setting_sources"],
            ),
            (
                "a floor control missing",
                partial,
                vec![],
                vec!["no_setting_sources"],
            ),
            ("the whole floor declared", full, vec![], vec![]),
            (
                "a template control beyond the floor",
                full,
                boundary.clone(),
                vec!["no_session_persistence"],
            ),
            (
                "a floor control repeated in the boundary",
                partial,
                vec!["no_setting_sources".to_string()],
                vec!["no_setting_sources"],
            ),
            ("everything declared", extra, boundary, vec![]),
        ] {
            let missing = manifest(declaration).unenforceable(&profile);
            assert_eq!(missing, expected, "{name}");
        }
    }

    /// spec: Task templates run without domain logic in the core runner
    /// A template the runner cannot execute is refused at parse, naming why; a sound one loads.
    #[test]
    fn a_malformed_template_is_refused_naming_why() {
        const BASE: &str = r#"
instructions = "Do the task."
[[fields]]
name = "n"
type = "integer"
[[fields]]
name = "s"
type = "string"
"#;
        let span_on_string = format!(
            "{BASE}\n[[rules]]\nkind = \"span\"\nquote_field = \"s\"\nturn_field = \"s\"\n"
        );
        let span_quote_integer = format!(
            "{BASE}\n[[rules]]\nkind = \"span\"\nquote_field = \"n\"\nturn_field = \"n\"\n"
        );
        let equals_integer =
            format!("{BASE}\n[[rules]]\nkind = \"equals\"\nfield = \"n\"\nvalue = \"1\"\n");
        let undeclared =
            format!("{BASE}\n[[rules]]\nkind = \"equals\"\nfield = \"missing\"\nvalue = \"x\"\n");
        let values_integer =
            format!("{BASE}\n[[fields]]\nname = \"t\"\ntype = \"integer\"\nvalues = [\"1\"]\n");
        let duplicate = format!("{BASE}\n[[fields]]\nname = \"s\"\ntype = \"string\"\n");
        for (name, text) in [
            ("empty instructions", BASE.replace("Do the task.", "   ")),
            ("no fields", "instructions = \"Do the task.\"\n".to_string()),
            ("a duplicate field", duplicate),
            ("a rule naming an undeclared field", undeclared),
            ("a span over a string turn field", span_on_string),
            ("a span quoting an integer field", span_quote_integer),
            ("a comparison on an integer field", equals_integer),
            ("enum values on an integer field", values_integer),
            (
                "an empty boundary name",
                BASE.replacen("[[fields]]", "boundary = [\"\"]\n[[fields]]", 1),
            ),
            (
                "an unknown declaration",
                format!("{BASE}\nunexpected = true\n"),
            ),
        ] {
            let error = TaskTemplate::parse(&text).err();
            assert!(error.is_some(), "{name} was accepted: {text}");
        }
        let sound = format!(
            "{}\n[[rules]]\nkind = \"span\"\nquote_field = \"s\"\nturn_field = \"n\"\n",
            BASE.replacen(
                "[[fields]]",
                "boundary = [\"no_session_persistence\"]\n[[fields]]",
                1
            )
        );
        TaskTemplate::parse(&sound).unwrap();
    }

    /// spec: Task templates run without domain logic in the core runner
    /// The output schema is the template's declarations: types, enum values, required and
    /// optional fields, and nothing the template does not declare.
    #[test]
    fn the_output_schema_reflects_the_declarations() {
        let template = TaskTemplate::parse(
            r#"
instructions = "Do the task."
[[fields]]
name = "section"
type = "integer"
[[fields]]
name = "title"
type = "string"
[[fields]]
name = "status"
type = "string"
values = ["published", "draft"]
[[fields]]
name = "note"
type = "string"
required = false
"#,
        )
        .unwrap();
        assert_eq!(
            template.output_schema(),
            json!({
                "type": "object",
                "properties": {"candidates": {"type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "section": {"type": "integer"},
                        "title": {"type": "string"},
                        "status": {"type": "string", "enum": ["published", "draft"]},
                        "note": {"type": ["string", "null"]},
                    },
                    "required": ["section", "title", "status"],
                    "additionalProperties": false,
                }}},
                "required": ["candidates"],
                "additionalProperties": false,
            })
        );
    }
}
