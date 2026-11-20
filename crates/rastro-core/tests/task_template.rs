//! The generic task runner against a real adapter process: a synthetic blog template, different
//! fields and text checks from the meeting template's, runs through the unchanged runner; a
//! control the adapter cannot enforce refuses the run before the adapter starts; a declared
//! exclusion counts a candidate without a retry. The adapter is a shell script that records
//! every launch and request and answers turn `n` from `response-<n>.json` (else
//! `response.json`), so no model is ever called.
use rastro_core::Locations;
use rastro_core::host::task::{self, TaskInput};
use rastro_plugin_sdk::{JudgmentTurn, TaskTemplate};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tempfile::TempDir;

const ADAPTER_ID: &str = "t1-task-adapter";

const MANIFEST: &str = r#"id = "t1-task-adapter"
name = "Rastro test task adapter"
version = "0.1.0"
author = "Rastro"
kinds = ["agent-tool-adapter"]
protocol = 1
command = "adapter"
agent_tool = { capabilities = ["judge_text"] }
enforcement = ["tools_off", "no_mcp_servers", "no_setting_sources"]

[permissions]
submit = []
scopes = []
env = []
actions = []
"#;

/// A synthetic blog task: fields and a text check different from the meeting template's, and
/// an exclusion the meeting template does not declare.
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
            text: "Drafts stay out of the published feed until their owner approves them.".into(),
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

struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        let dir = env.path(&format!("config/plugins/{ADAPTER_ID}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("rastro-plugin.toml"), MANIFEST).unwrap();
        // The script records to absolute paths baked in here, so it needs no environment of
        // its own beyond the basics the host gives every plugin.
        let script = format!(
            r#"#!/bin/sh
echo launch >> '{dir}/launches'
printf '0\n' > '{dir}/turns'
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocol":"1.0","manifest_id":"{ADAPTER_ID}"}}}}\n' "$id"
      ;;
    *'"method":"judgment.request"'*)
      n=$(( $(cat '{dir}/turns' 2>/dev/null || echo 0) + 1 ))
      printf '%s\n' "$n" > '{dir}/turns'
      printf '%s\n' "$line" > "{dir}/judgment-$n.json"
      printf '{{"jsonrpc":"2.0","id":%s,"result":' "$id"
      if [ -f "{dir}/response-$n.json" ]; then
        cat "{dir}/response-$n.json"
      else
        cat '{dir}/response.json'
      fi
      printf '}}\n'
      ;;
  esac
done
"#,
            dir = dir.display()
        );
        fs::write(dir.join("adapter"), script).unwrap();
        fs::set_permissions(dir.join("adapter"), fs::Permissions::from_mode(0o755)).unwrap();
        env.respond(json!({ "candidates": [published()] }));
        env
    }
    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }
    fn dir(&self) -> PathBuf {
        self.path(&format!("config/plugins/{ADAPTER_ID}"))
    }
    fn locations(&self) -> Locations {
        Locations {
            data: self.path("data"),
            config: self.path("config"),
        }
    }
    fn manifest(&self, text: &str) {
        fs::write(
            self.path(&format!("config/plugins/{ADAPTER_ID}/rastro-plugin.toml")),
            text,
        )
        .unwrap();
    }
    fn respond(&self, result: Value) {
        fs::write(self.dir().join("response.json"), result.to_string()).unwrap();
    }
    fn respond_turn(&self, n: u64, result: Value) {
        fs::write(
            self.dir().join(format!("response-{n}.json")),
            result.to_string(),
        )
        .unwrap();
    }
    fn judgment(&self, n: u64) -> Value {
        let text = fs::read_to_string(self.dir().join(format!("judgment-{n}.json"))).unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn turns(&self) -> u64 {
        fs::read_to_string(self.dir().join("turns"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
    fn launches(&self) -> usize {
        fs::read_to_string(self.dir().join("launches"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }
    fn run(&self, template: &TaskTemplate) -> Result<task::TaskOutput, task::TaskFailure> {
        let turns = sections();
        task::run(
            &self.locations(),
            ADAPTER_ID,
            template,
            &TaskInput {
                owner_speaker: "",
                turns: &turns,
            },
            3,
            Duration::from_secs(5),
        )
    }
}

/// spec: Task templates run without domain logic in the core runner
/// S1: the unchanged runner evaluates the blog declarations and returns the validated
/// blog-shaped result; the request carries the blog's own instructions and fields.
#[test]
fn the_runner_returns_validated_blog_shaped_results() {
    let env = Env::new();
    let second = json!({
        "section": 2,
        "title": "Drafts wait",
        "excerpt": "Drafts stay out of the published feed",
        "status": "published"
    });
    env.respond(json!({ "candidates": [published(), second] }));
    let output = env.run(&TaskTemplate::parse(BLOG).unwrap()).unwrap();
    assert_eq!(output.candidates, vec![published(), second]);
    assert_eq!(output.excluded, 0);
    assert_eq!(output.history.len(), 1);
    assert_eq!(output.history[0].verdict, r#"{"passed":true}"#);
    assert_eq!(env.launches(), 1);
    assert_eq!(env.turns(), 1);
    let request = env.judgment(1);
    assert_eq!(
        request["params"]["instructions"],
        json!("Propose blog summaries from the labelled sections: a title and an exact excerpt.")
    );
    assert_eq!(
        request["params"]["fields"],
        json!(["section", "title", "excerpt", "status"])
    );
    assert_eq!(request["params"]["turns"].as_array().unwrap().len(), 2);
}

/// spec: Task templates run without domain logic in the core runner
/// S2: a control the adapter cannot enforce — one of the core floor's or one the task's
/// boundary demands — refuses the run naming it, before the adapter starts or sees any text.
#[test]
fn an_unenforceable_control_refuses_the_run_before_the_adapter_sees_text() {
    let floor_line = "enforcement = [\"tools_off\", \"no_mcp_servers\", \"no_setting_sources\"]";
    let partial = "enforcement = [\"tools_off\", \"no_mcp_servers\"]";
    for (name, manifest, boundary, missing) in [
        (
            "a floor control",
            MANIFEST.replace(floor_line, partial),
            vec![],
            "no_setting_sources",
        ),
        (
            "a control the task boundary demands",
            MANIFEST.to_string(),
            vec!["no_session_persistence".to_string()],
            "no_session_persistence",
        ),
    ] {
        let env = Env::new();
        env.manifest(&manifest);
        let text = if boundary.is_empty() {
            BLOG.to_string()
        } else {
            BLOG.replacen(
                "[[fields]]",
                "boundary = [\"no_session_persistence\"]\n[[fields]]",
                1,
            )
        };
        let template = TaskTemplate::parse(&text).unwrap();
        let failure = env.run(&template).err();
        let reason = failure
            .unwrap_or_else(|| panic!("{name}: the run was accepted"))
            .reason;
        assert!(
            reason.contains(missing),
            "{name}: {missing} not named: {reason}"
        );
        assert_eq!(env.launches(), 0, "{name}: the adapter started");
        assert!(
            !env.dir().join("judgment-1.json").exists(),
            "{name}: the adapter received a request"
        );
    }
}

/// spec: Task templates run without domain logic in the core runner
/// A gate failure earns a retry turn of the same session carrying the core's feedback and the
/// harness session id the first turn produced (ADR-0016 §4, §8).
#[test]
fn a_gate_failure_earns_a_retry_turn_in_the_same_session() {
    let env = Env::new();
    let mut broken = published();
    broken["excerpt"] = json!("words the section never says");
    env.respond_turn(1, json!({ "candidates": [broken], "session_id": "sess-1" }));
    env.respond_turn(
        2,
        json!({ "candidates": [published()], "session_id": "sess-2" }),
    );
    let output = env.run(&TaskTemplate::parse(BLOG).unwrap()).unwrap();
    assert_eq!(output.candidates, vec![published()]);
    assert_eq!(output.history.len(), 2);
    assert_eq!(env.launches(), 1, "the retry opened a new process");
    assert_eq!(env.turns(), 2);
    assert!(env.judgment(1)["params"]["session"].is_null());
    let verdict: Value = serde_json::from_str(&output.history[0].verdict).unwrap();
    assert_eq!(verdict["passed"], json!(false));
    assert_eq!(verdict["feedback"][0]["gate"], json!("span"));
    let retry = env.judgment(2);
    assert_eq!(retry["params"]["session"]["turn"], json!(2));
    assert_eq!(
        retry["params"]["session"]["feedback"][0]["gate"],
        json!("span")
    );
    assert_eq!(
        retry["params"]["session"]["prior_answer"]["session_id"],
        json!("sess-1"),
        "the retry resumes the session the first turn produced"
    );
}

/// spec: Task templates run without domain logic in the core runner
/// S3: a valid candidate matching a declared exclusion is counted as excluded without causing
/// a retry: the adapter answers exactly one turn and the kept candidate passes.
#[test]
fn a_declared_exclusion_counts_the_candidate_without_a_retry() {
    let env = Env::new();
    let mut draft = published();
    draft["status"] = json!("draft");
    env.respond(json!({ "candidates": [draft, published()] }));
    let output = env.run(&TaskTemplate::parse(BLOG).unwrap()).unwrap();
    assert_eq!(output.excluded, 1);
    assert_eq!(output.candidates, vec![published()]);
    assert_eq!(output.history.len(), 1);
    assert_eq!(output.history[0].verdict, r#"{"passed":true}"#);
    assert_eq!(env.turns(), 1, "the exclusion caused a retry turn");
    assert_eq!(env.launches(), 1);
}
