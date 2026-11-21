//! The generic task runner: one adapter session under a task template's declared rules
//! (ADR-0016). The runner knows no task: it sends the template's own instructions and field
//! names, executes the template's declared gates on every answer, and counts the candidates a
//! declared exclusion matches. Gate failures earn a retry turn of the same capped session;
//! the core owns the trust boundary, the retries and the recording. Meeting preparation and
//! mapping stay outside the runner (decisions/plugins.md §40).
use super::{JudgmentFailure, JudgmentSession};
use crate::{Locations, display_safe};
use rastro_plugin_sdk::{
    GateFeedback, JudgmentRequest, JudgmentSession as WireSession, JudgmentTurn, RelativeTurn,
    RuleDisposition, TaskFieldType, TaskPredicate, TaskRule, TaskTemplate, TaskText,
};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, time::Duration};

/// What one task run judges: the turns of untrusted text and the main label (empty when the
/// task has no main label). The optional label set is trusted context for validation only;
/// it is never sent to the adapter.
pub struct TaskInput<'a> {
    pub owner_speaker: &'a str,
    pub turns: &'a [JudgmentTurn],
    pub labels: Option<&'a BTreeSet<String>>,
}

/// One turn of a task session: the request the core sent, the answer it got and the gate
/// verdict, failed turns included (ADR-0016 §3).
#[derive(Debug)]
pub struct TaskTurn {
    pub turn: u64,
    pub request: String,
    pub answer: Option<String>,
    pub verdict: String,
}

/// What a passing run produced: the validated candidates, exactly the template's fields, the
/// count a declared exclusion matched, and every turn's history.
#[derive(Debug)]
pub struct TaskOutput {
    pub candidates: Vec<CheckedCandidate>,
    pub excluded: usize,
    pub history: Vec<TaskTurn>,
}

/// A candidate that passed the rules. Keep the original proposal for replay: grounding only
/// changes which fields a consumer may use.
#[derive(Debug)]
pub struct CheckedCandidate {
    pub proposed: Value,
    pub cleared_fields: Vec<String>,
}

/// Why a run produced no validated output: a refused or broken adapter, or a cap exhausted
/// with gate failures. The history holds every turn the session ran.
#[derive(Debug)]
pub struct TaskFailure {
    pub reason: String,
    pub history: Vec<TaskTurn>,
    /// The gate whose failure exhausted the turn cap, when gate failures ended the run; `None`
    /// when the adapter itself broke or was refused (a system failure).
    pub gate: Option<String>,
    /// The adapter failure that ended the run, when it was a system failure rather than gate
    /// failures exhausting the cap.
    pub system: Option<JudgmentFailure>,
}

/// Runs the task template over the input with the installed adapter `adapter`: at most `cap`
/// turns, each bounded by `timeout`. The adapter's declared enforcement must match the
/// judgment floor and the template's boundary before the adapter starts; a run that cannot
/// enforce a required control is refused without the adapter seeing any text.
pub fn run(
    locations: &Locations,
    adapter: &str,
    template: &TaskTemplate,
    input: &TaskInput<'_>,
    cap: u64,
    timeout: Duration,
) -> Result<TaskOutput, TaskFailure> {
    let mut session = match JudgmentSession::open_with_boundary(
        locations,
        adapter,
        timeout,
        &template.boundary,
    ) {
        Ok(session) => session,
        Err(failure) => {
            return Err(TaskFailure {
                reason: failure_reason(&failure),
                history: Vec::new(),
                gate: None,
                system: Some(failure),
            });
        }
    };
    let outcome = run_turns(&mut session, template, input, cap);
    session.close(outcome.is_ok());
    outcome
}

/// The session loop: ask, gate, and on a gate failure ask again with the core's feedback
/// while turns remain under the cap (ADR-0016 §8). A system failure on any turn fails the run.
fn run_turns(
    session: &mut JudgmentSession,
    template: &TaskTemplate,
    input: &TaskInput<'_>,
    cap: u64,
) -> Result<TaskOutput, TaskFailure> {
    let mut history: Vec<TaskTurn> = Vec::new();
    let mut request = JudgmentRequest {
        instructions: template.instructions.clone(),
        owner_speaker: input.owner_speaker.to_string(),
        turns: input.turns.to_vec(),
        fields: template
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect(),
        output_schema: Some(template.output_schema()),
        session: None,
    };
    let mut feedback: Vec<GateFeedback> = Vec::new();
    let mut prior: Option<Value> = None;
    let mut session_id: Option<String> = None;
    for turn in 1..=cap {
        if let Some(value) = &prior {
            request.session = Some(WireSession {
                turn,
                feedback: feedback.clone(),
                prior_answer: prior_answer_of(value, session_id.as_deref()),
            });
        }
        let sent = serde_json::to_string(&request)
            .expect("the judgment request is plain data and serialises");
        let value = match session.turn(&request) {
            Ok(value) => value,
            Err(failure) => {
                let reason = failure_reason(&failure);
                history.push(TaskTurn {
                    turn,
                    request: sent,
                    answer: None,
                    verdict: system_verdict(&reason),
                });
                return Err(TaskFailure {
                    reason,
                    history,
                    gate: None,
                    system: Some(failure),
                });
            }
        };
        // Persist the first harness session id the session produced: every retry turn resumes
        // it, even when a later answer omits it (ADR-0016 §4).
        if session_id.is_none() {
            session_id = value
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string);
        }
        let answer = value.to_string();
        match evaluate(template, input, &value) {
            Evaluation::Pass {
                candidates,
                excluded,
            } => {
                history.push(TaskTurn {
                    turn,
                    request: sent,
                    answer: Some(answer),
                    verdict: PASS_VERDICT.into(),
                });
                return Ok(TaskOutput {
                    candidates,
                    excluded,
                    history,
                });
            }
            Evaluation::Retry(found) => {
                history.push(TaskTurn {
                    turn,
                    request: sent,
                    answer: Some(answer),
                    verdict: gate_verdict(&found),
                });
                feedback = found;
                prior = Some(value);
            }
        }
    }
    // The cap is exhausted: the last answer's gate failures name the cause and the reason.
    let gate = feedback.first().map(|f| f.gate.as_str()).unwrap_or("shape");
    Err(TaskFailure {
        reason: format!(
            "the turn cap of {cap} is exhausted; the last answer fails the {gate} gate: {}",
            feedback_details(&feedback)
        ),
        history,
        gate: Some(gate.to_string()),
        system: None,
    })
}

/// Re-checks recorded output against a saved contract with no model call: the same shape and
/// rule gates a run executes, over the turns the record retains (decisions/plugins.md §48).
/// Returns the kept candidates and the count a declared exclusion matched, or the gate failures
/// the record no longer passes.
pub fn check(
    template: &TaskTemplate,
    input: &TaskInput<'_>,
    answer: &Value,
) -> Result<(Vec<CheckedCandidate>, usize), Vec<GateFeedback>> {
    match evaluate(template, input, answer) {
        Evaluation::Pass {
            candidates,
            excluded,
        } => Ok((candidates, excluded)),
        Evaluation::Retry(feedback) => Err(feedback),
    }
}

/// The prior answer a retry turn carries: the whole recorded response in the task's own
/// shape, so a resumed or degraded turn corrects the candidates it actually proposed
/// (ADR-0016 §4, §8). The persisted harness session id wins over whatever the answer names;
/// an answer that is not an object rides along unchanged, with the feedback naming what was
/// there.
fn prior_answer_of(value: &Value, session_id: Option<&str>) -> Value {
    let mut prior = value.clone();
    if let (Some(id), Some(object)) = (session_id, prior.as_object_mut()) {
        object.insert("session_id".into(), Value::String(id.to_string()));
    }
    prior
}

/// What the core's gates made of one answer: the validated candidates and the excluded count,
/// or the gate failures a retry turn must fix (ADR-0016 §8).
enum Evaluation {
    Pass {
        candidates: Vec<CheckedCandidate>,
        excluded: usize,
    },
    Retry(Vec<GateFeedback>),
}

/// Executes the template's declarations on one answer: the shape its fields declare, then its
/// rules over each candidate. Pure: no process, no storage, the same gates a replay re-runs.
fn evaluate(template: &TaskTemplate, input: &TaskInput<'_>, result: &Value) -> Evaluation {
    let mut feedback = Vec::new();
    let candidates = envelope_shape(result, &mut feedback);
    for (n, candidate) in candidates.iter().enumerate() {
        candidate_shape(template, candidate, n, &mut feedback);
    }
    if !feedback.is_empty() {
        return Evaluation::Retry(feedback);
    }
    let mut kept = Vec::new();
    let mut excluded = 0;
    for (n, candidate) in candidates.iter().enumerate() {
        let Some(object) = candidate.as_object() else {
            continue; // the shape gate already refused a non-object
        };
        let rules = CandidateRules { input, object };
        let mut out = false;
        let mut cleared_fields = Vec::new();
        for rule in &template.rules {
            let (disposition, triggered) = match rule {
                TaskRule::Span {
                    quote_field,
                    turn_field,
                    disposition,
                } => (
                    *disposition,
                    span_failure(input.turns, object, quote_field, turn_field, n),
                ),
                TaskRule::Equals {
                    field,
                    value,
                    disposition,
                } => (
                    *disposition,
                    (rules.field(field) == Some(value.as_str())).then(|| GateFeedback {
                        gate: "equals".into(),
                        field: format!("candidates[{n}].{field}"),
                        expected: format!("a value other than {value:?}"),
                        actual: display_safe(&object[field].to_string()),
                    }),
                ),
                TaskRule::Require { predicate } => {
                    out |= !rules.matches(predicate);
                    continue;
                }
                TaskRule::Ground { field, in_field } => {
                    if let Some(value) = rules.field(field)
                        && (value.trim().is_empty()
                            || !rules
                                .field(in_field)
                                .is_some_and(|text| text.contains(value)))
                    {
                        cleared_fields.push(field.clone());
                    }
                    continue;
                }
            };
            // Exclusion cannot mask a gate failure. All rules read the original answer.
            if let Some(failure) = triggered {
                match disposition {
                    RuleDisposition::Fail => feedback.push(failure),
                    RuleDisposition::Exclude => out = true,
                }
            }
        }
        if out {
            excluded += 1;
        } else {
            kept.push(CheckedCandidate {
                proposed: (*candidate).clone(),
                cleared_fields,
            });
        }
    }
    if feedback.is_empty() {
        Evaluation::Pass {
            candidates: kept,
            excluded,
        }
    } else {
        Evaluation::Retry(feedback)
    }
}

struct CandidateRules<'a> {
    input: &'a TaskInput<'a>,
    object: &'a Map<String, Value>,
}
impl CandidateRules<'_> {
    fn field(&self, name: &str) -> Option<&str> {
        self.object.get(name).and_then(Value::as_str)
    }

    fn text(&self, source: &TaskText) -> Option<&str> {
        match source {
            TaskText::Field { name } => self.field(name).filter(|s| !s.trim().is_empty()),
            TaskText::InputLabel => Some(self.input.owner_speaker).filter(|s| !s.trim().is_empty()),
            TaskText::TurnLabel { turn_field, at } => {
                let index = self.object.get(turn_field)?.as_u64()?.checked_sub(1)?;
                let index = match at {
                    RelativeTurn::Previous => index.checked_sub(1)?,
                    RelativeTurn::Current => index,
                    RelativeTurn::Next => index.checked_add(1)?,
                };
                Some(
                    self.input
                        .turns
                        .get(usize::try_from(index).ok()?)?
                        .speaker
                        .as_str(),
                )
                .filter(|label| !label.is_empty())
            }
        }
    }

    fn matches(&self, predicate: &TaskPredicate) -> bool {
        match predicate {
            TaskPredicate::All { of } => of.iter().all(|item| self.matches(item)),
            TaskPredicate::Any { of } => of.iter().any(|item| self.matches(item)),
            TaskPredicate::Not { of } => !self.matches(of),
            TaskPredicate::Equal { left, right } => self
                .text(left)
                .is_some_and(|left| Some(left) == self.text(right)),
            TaskPredicate::ContainsLabel { field, label } => self
                .field(field)
                .zip(self.text(label))
                .is_some_and(|(text, label)| contains_label(text, label)),
            TaskPredicate::ContainsWord { field, words } => self
                .field(field)
                .is_some_and(|text| contains_word(text, words)),
            TaskPredicate::InInputLabels { field } => self
                .field(field)
                .zip(self.input.labels)
                .is_some_and(|(value, labels)| labels.contains(value)),
        }
    }
}

fn contains_label(text: &str, label: &str) -> bool {
    let (text, label) = (text.to_lowercase(), label.trim().to_lowercase());
    if label.is_empty() {
        return false;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(&label).any(|(start, found)| {
        !text[..start].chars().next_back().is_some_and(word)
            && !text[start + found.len()..].chars().next().is_some_and(word)
    })
}

fn contains_word(text: &str, words: &[String]) -> bool {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
        .any(|word| words.contains(&word.replace('\u{2019}', "'")))
}

/// The protocol's own envelope: an object holding the candidates, the harness session id and
/// the degradation flag, and nothing else. Returns the candidate list to check.
fn envelope_shape<'a>(result: &'a Value, feedback: &mut Vec<GateFeedback>) -> Vec<&'a Value> {
    let Some(object) = result.as_object() else {
        feedback.push(GateFeedback {
            gate: "shape".into(),
            field: "result".into(),
            expected: "an object with the candidates".into(),
            actual: display_safe(&result.to_string()),
        });
        return Vec::new();
    };
    for (key, value) in object {
        if !matches!(key.as_str(), "candidates" | "session_id" | "degraded") {
            feedback.push(shape_feedback(
                key,
                "only the fields candidates, session_id, degraded",
                value,
            ));
        }
    }
    let mut candidates = Vec::new();
    match object.get("candidates") {
        None => feedback.push(GateFeedback {
            gate: "shape".into(),
            field: "candidates".into(),
            expected: "a list of candidates".into(),
            actual: "missing".into(),
        }),
        Some(Value::Array(list)) => candidates = list.iter().collect(),
        Some(value) => feedback.push(shape_feedback("candidates", "a list of candidates", value)),
    }
    if let Some(value) = object
        .get("session_id")
        .filter(|v| !v.is_string() && !v.is_null())
    {
        feedback.push(shape_feedback(
            "session_id",
            "the harness session id as text",
            value,
        ));
    }
    if let Some(value) = object
        .get("degraded")
        .filter(|v| !v.is_boolean() && !v.is_null())
    {
        feedback.push(shape_feedback("degraded", "true or false", value));
    }
    candidates
}

/// One candidate's shape: exactly the fields the template declares, the required ones
/// present, each the type and value its declaration allows.
fn candidate_shape(
    template: &TaskTemplate,
    candidate: &Value,
    n: usize,
    feedback: &mut Vec<GateFeedback>,
) {
    let path = |name: &str| format!("candidates[{n}].{name}");
    let Some(object) = candidate.as_object() else {
        feedback.push(shape_feedback(
            &format!("candidates[{n}]"),
            "an object",
            candidate,
        ));
        return;
    };
    let names = template
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    for (key, value) in object {
        if template.field(key).is_none() {
            feedback.push(shape_feedback(
                &path(key),
                &format!("only the fields {names}"),
                value,
            ));
        }
    }
    for declared in &template.fields {
        let expected = match declared.field_type {
            TaskFieldType::Integer => "a non-negative whole number".to_string(),
            TaskFieldType::String if !declared.values.is_empty() => {
                format!("one of `{}`", declared.values.join("`, `"))
            }
            TaskFieldType::String if declared.required => "a non-empty string".into(),
            TaskFieldType::String => "a string".into(),
        };
        let violation = match object.get(&declared.name) {
            None => declared.required,
            Some(Value::Null) => declared.required,
            Some(value) => match declared.field_type {
                TaskFieldType::Integer => value.as_u64().is_none(),
                TaskFieldType::String => !value.as_str().is_some_and(|text| {
                    (!declared.required || !text.trim().is_empty())
                        && (declared.values.is_empty()
                            || declared.values.iter().any(|allowed| allowed == text))
                }),
            },
        };
        if violation {
            let actual = object
                .get(&declared.name)
                .map(|value| display_safe(&value.to_string()))
                .unwrap_or_else(|| "missing".into());
            feedback.push(GateFeedback {
                gate: "shape".into(),
                field: path(&declared.name),
                expected,
                actual,
            });
        }
    }
}

/// The span rule's failure, if the quote is not an exact, contiguous, non-empty piece of the
/// text of the turn the turn field names.
fn span_failure(
    turns: &[JudgmentTurn],
    object: &serde_json::Map<String, Value>,
    quote_field: &str,
    turn_field: &str,
    n: usize,
) -> Option<GateFeedback> {
    let named = object.get(turn_field).and_then(Value::as_u64);
    let Some(turn) = named
        .and_then(|turn| turn.checked_sub(1))
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| turns.get(index))
    else {
        return Some(GateFeedback {
            gate: "span".into(),
            field: format!("candidates[{n}].{turn_field}"),
            expected: format!("a turn of the input (1..={})", turns.len()),
            actual: named
                .map(|turn| turn.to_string())
                .unwrap_or_else(|| "missing".into()),
        });
    };
    let quote = object
        .get(quote_field)
        .and_then(Value::as_str)
        .unwrap_or_default();
    if quote.trim().is_empty() || !turn.text.contains(quote) {
        return Some(GateFeedback {
            gate: "span".into(),
            field: format!("candidates[{n}].{quote_field}"),
            expected: format!("an exact, contiguous piece of turn {}'s text", turn.turn),
            actual: display_safe(quote),
        });
    }
    None
}

/// One shape violation: the field it was found at, what the shape expects there, and what the
/// answer held, bounded and escaped like any plugin text.
fn shape_feedback(field: &str, expected: &str, value: &Value) -> GateFeedback {
    GateFeedback {
        gate: "shape".into(),
        field: field.into(),
        expected: expected.into(),
        actual: display_safe(&value.to_string()),
    }
}

/// The verdict a passing turn's history keeps.
const PASS_VERDICT: &str = r#"{"passed":true}"#;

/// A gate failure's verdict: the core-authored feedback the retry turn carries, as JSON.
fn gate_verdict(feedback: &[GateFeedback]) -> String {
    serde_json::to_string(&serde_json::json!({ "passed": false, "feedback": feedback }))
        .expect("gate feedback is plain data and serialises")
}

/// A system failure's verdict: no answer arrived; the reason names what broke.
fn system_verdict(reason: &str) -> String {
    serde_json::to_string(&serde_json::json!({ "passed": false, "reason": reason }))
        .expect("a verdict reason is plain data and serialises")
}

/// The gate failures as one deterministic line: field, expected, actual, in the order the
/// gates found them.
fn feedback_details(feedback: &[GateFeedback]) -> String {
    feedback
        .iter()
        .map(|f| format!("{}: expected {}, actual {}", f.field, f.expected, f.actual))
        .collect::<Vec<_>>()
        .join("; ")
}

/// A session failure's reason: the adapter broke, or its answer was not the protocol's shape.
fn failure_reason(failure: &JudgmentFailure) -> String {
    match failure {
        JudgmentFailure::Failed(reason) | JudgmentFailure::Invalid(reason) => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A synthetic blog task: fields and a text check different from the meeting template's,
    /// and an exclusion the meeting template does not declare.
    const BLOG: &str = r#"
instructions = "Propose blog summaries from the labelled sections: a title and an exact excerpt."

[[fields]]
name = "section"
type = "integer"

[[fields]]
name = "title"
type = "string"

[[fields]]
name = "excerpt"
type = "string"

[[fields]]
name = "status"
type = "string"
values = ["published", "draft"]

[[rules]]
kind = "span"
quote_field = "excerpt"
turn_field = "section"

[[rules]]
kind = "equals"
field = "status"
value = "draft"
disposition = "exclude"
"#;

    fn blog() -> TaskTemplate {
        TaskTemplate::parse(BLOG).unwrap()
    }

    fn check_blog(template: &TaskTemplate, answer: &Value) -> Evaluation {
        let input = TaskInput {
            owner_speaker: "",
            turns: &sections(),
            labels: None,
        };
        evaluate(template, &input, answer)
    }

    fn proposed(candidates: Vec<CheckedCandidate>) -> Vec<Value> {
        candidates
            .into_iter()
            .map(|candidate| candidate.proposed)
            .collect()
    }

    fn sections() -> Vec<JudgmentTurn> {
        vec![
            JudgmentTurn {
                turn: 1,
                speaker: "post".into(),
                text: "Rastro keeps every quote exact. A hypothesis is not a confirmed commitment."
                    .into(),
            },
            JudgmentTurn {
                turn: 2,
                speaker: "post".into(),
                text: "Drafts stay out of the published feed until their owner approves them."
                    .into(),
            },
        ]
    }

    fn published() -> Value {
        json!({
            "section": 1,
            "title": "Exact quotes",
            "excerpt": "Rastro keeps every quote exact.",
            "status": "published"
        })
    }

    /// spec: Task templates run without domain logic in the core runner
    /// The runner evaluates the blog declarations and keeps the blog-shaped candidates.
    #[test]
    fn blog_declarations_validate_a_blog_shaped_answer() {
        let second = json!({
            "section": 2,
            "title": "Drafts wait",
            "excerpt": "Drafts stay out of the published feed",
            "status": "published"
        });
        let answer = json!({ "candidates": [published(), second] });
        let Evaluation::Pass {
            candidates,
            excluded,
        } = check_blog(&blog(), &answer)
        else {
            panic!("the blog answer failed its own declarations: {answer}");
        };
        assert_eq!(proposed(candidates), vec![published(), second]);
        assert_eq!(excluded, 0);
    }

    /// spec: Task templates run without domain logic in the core runner
    /// An optional field may be absent or null; a present one still meets its type.
    #[test]
    fn an_optional_field_may_be_absent_or_null() {
        let template = TaskTemplate::parse(&format!(
            "{BLOG}\n[[fields]]\nname = \"note\"\ntype = \"string\"\nrequired = false\n"
        ))
        .unwrap();
        let mut absent = published();
        absent["note"] = Value::Null;
        let answer = json!({ "candidates": [published(), absent] });
        let Evaluation::Pass {
            candidates,
            excluded,
        } = check_blog(&template, &answer)
        else {
            panic!("an optional field was refused: {answer}");
        };
        assert_eq!(candidates.len(), 2);
        assert_eq!(excluded, 0);
    }

    /// spec: Task templates run without domain logic in the core runner
    /// S3: a valid candidate a declared exclusion matches is counted, never feedback; the
    /// candidates it does not match pass untouched.
    #[test]
    fn a_declared_exclusion_counts_without_feedback() {
        let mut draft = published();
        draft["status"] = json!("draft");
        let answer = json!({ "candidates": [draft, published()] });
        let Evaluation::Pass {
            candidates,
            excluded,
        } = check_blog(&blog(), &answer)
        else {
            panic!("the exclusion caused a retry: {answer}");
        };
        assert_eq!(excluded, 1);
        assert_eq!(proposed(candidates), vec![published()]);
    }

    /// spec: Task templates run without domain logic in the core runner
    /// The comparison primitive's fail disposition earns retry feedback naming the field.
    #[test]
    fn an_equals_failure_becomes_retry_feedback() {
        let template = TaskTemplate::parse(&BLOG.replacen(
            "disposition = \"exclude\"",
            "disposition = \"fail\"",
            1,
        ))
        .unwrap();
        let mut draft = published();
        draft["status"] = json!("draft");
        let answer = json!({ "candidates": [draft] });
        let Evaluation::Retry(feedback) = check_blog(&template, &answer) else {
            panic!("the equals rule did not fail the candidate: {answer}");
        };
        assert!(
            feedback
                .iter()
                .any(|f| f.gate == "equals" && f.field == "candidates[0].status"),
            "{feedback:?}"
        );
    }

    /// spec: Task templates run without domain logic in the core runner
    /// Every shape violation becomes core-authored retry feedback naming the field.
    #[test]
    fn shape_violations_become_retry_feedback() {
        let mut unknown = published();
        unknown["kind"] = json!("promise");
        let mut missing = published();
        missing.as_object_mut().unwrap().remove("title");
        let mut mistyped = published();
        mistyped["section"] = json!("1");
        let mut unlisted = published();
        unlisted["status"] = json!("archived");
        let mut empty = published();
        empty["title"] = json!("   ");
        for (name, answer, field) in [
            (
                "an undeclared field",
                json!({ "candidates": [unknown] }),
                "candidates[0].kind",
            ),
            (
                "a missing required field",
                json!({ "candidates": [missing] }),
                "candidates[0].title",
            ),
            (
                "a mistyped field",
                json!({ "candidates": [mistyped] }),
                "candidates[0].section",
            ),
            (
                "a value outside the enum",
                json!({ "candidates": [unlisted] }),
                "candidates[0].status",
            ),
            (
                "an empty required string",
                json!({ "candidates": [empty] }),
                "candidates[0].title",
            ),
            (
                "candidates not a list",
                json!({ "candidates": {} }),
                "candidates",
            ),
        ] {
            let Evaluation::Retry(feedback) = check_blog(&blog(), &answer) else {
                panic!("{name} passed the shape gate: {answer}");
            };
            assert!(!feedback.is_empty(), "{name}: no feedback");
            assert!(
                feedback.iter().all(|f| f.gate == "shape"),
                "{name}: {feedback:?}"
            );
            assert!(
                feedback.iter().any(|f| f.field == field),
                "{name}: {field} not named: {feedback:?}"
            );
            assert!(
                feedback
                    .iter()
                    .all(|f| !f.expected.is_empty() && !f.actual.is_empty()),
                "{name}: feedback without expected/actual: {feedback:?}"
            );
        }
    }

    /// spec: Task templates run without domain logic in the core runner
    /// The declared text check fails retryably when the excerpt is not an exact, contiguous
    /// piece of the section it names.
    #[test]
    fn span_failures_become_retry_feedback() {
        let mut zero = published();
        zero["section"] = json!(0);
        let mut past = published();
        past["section"] = json!(3);
        let mut absent = published();
        absent["excerpt"] = json!("words the section never says");
        for (name, answer, field) in [
            (
                "section zero",
                json!({ "candidates": [zero] }),
                "candidates[0].section",
            ),
            (
                "a section past the input",
                json!({ "candidates": [past] }),
                "candidates[0].section",
            ),
            (
                "an excerpt not in the section",
                json!({ "candidates": [absent] }),
                "candidates[0].excerpt",
            ),
        ] {
            let Evaluation::Retry(feedback) = check_blog(&blog(), &answer) else {
                panic!("{name} passed the span rule: {answer}");
            };
            assert!(
                feedback
                    .iter()
                    .any(|f| f.gate == "span" && f.field == field),
                "{name}: {field} not named by the span gate: {feedback:?}"
            );
        }
    }
}
