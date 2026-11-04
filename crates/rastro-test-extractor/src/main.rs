//! An extractor driven by a script, so tests can make it submit valid and invalid claims.
//!
//! `RASTRO_TEST_EXTRACTOR_SCRIPT` names a JSONL file; each line is one step, tagged by `do`:
//! - `initialize`: read the core's `initialize` and answer it
//! - `read_batches`: read `evidence.batch` notifications up to the one marked `last`
//! - `state_claims`: for every `/state` item read, a `code:state` claim on its location
//! - `send` {message}: write that JSON object as one line; in it, `"$evidence:N"` becomes the
//!   id of the Nth item read and `"$end:N"` that item's span end
//! - `finish` {status?}: send `run.finish` with the claims sent so far and wait for the reply
//! - `touch` {path}: create that file
//! - `drain`: read until the core closes the pipe
//! - `exit` {code?}: end the process at once
//!
//! Without a script it initializes, reads every batch, sends state claims and finishes.
use anyhow::{Context, Result, bail};
use rastro_plugin_sdk::{
    BatchItem, CLAIM_SUBMIT, Envelope, EvidenceBatch, ExtractorInitializeParams, InitializeResult,
    PROTOCOL_VERSION, RUN_FINISH, Role, RunFinish, Transport,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{env, fs, io};

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Step {
    Initialize,
    ReadBatches,
    StateClaims,
    Send {
        message: Value,
    },
    Finish {
        #[serde(default = "ok")]
        status: String,
    },
    Touch {
        path: String,
    },
    /// Pause one source after earlier sources have staged their generation; optionally fail.
    Gate {
        source: String,
        entered: String,
        release: String,
        #[serde(default)]
        fail: bool,
    },
    Drain,
    Exit {
        #[serde(default)]
        code: i32,
    },
}
fn ok() -> String {
    "ok".into()
}

fn script() -> Result<Vec<Step>> {
    let Some(path) =
        env::var_os("RASTRO_TEST_EXTRACTOR_SCRIPT").filter(|p| fs::metadata(p).is_ok())
    else {
        return Ok(serde_json::from_value(json!([
            {"do": "initialize"}, {"do": "read_batches"}, {"do": "state_claims"}, {"do": "finish"}
        ]))?);
    };
    fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).context("invalid script step"))
        .collect()
}

/// Replaces `"$evidence:N"` and `"$end:N"` with numbers from the items read.
fn fill(value: Value, items: &[BatchItem]) -> Value {
    match value {
        Value::String(text) => {
            let item = |prefix: &str| {
                let n: usize = text.strip_prefix(prefix)?.parse().ok()?;
                items.get(n)
            };
            if let Some(item) = item("$evidence:") {
                json!(item.evidence_id)
            } else if let Some(item) = item("$end:") {
                json!(item.span.end)
            } else {
                Value::String(text)
            }
        }
        Value::Array(values) => values.into_iter().map(|v| fill(v, items)).collect(),
        Value::Object(map) => map.into_iter().map(|(k, v)| (k, fill(v, items))).collect(),
        other => other,
    }
}

fn main() -> Result<()> {
    let mut transport = Transport::new(io::stdin().lock(), io::stdout().lock());
    let mut items: Vec<BatchItem> = Vec::new();
    let mut sent = 0;
    let mut source = String::new();
    for step in script()? {
        match step {
            Step::Initialize => {
                let request = transport.read()?.context("core closed before initialize")?;
                let params: ExtractorInitializeParams = request.params()?;
                source = params
                    .run
                    .pairs
                    .first()
                    .context("missing source binding")?
                    .source
                    .clone();
                if params.role != Role::Extractor {
                    bail!("not started as an extractor");
                }
                transport.send(&Envelope::response(
                    request.id,
                    InitializeResult {
                        protocol: PROTOCOL_VERSION.into(),
                        manifest_id: params.manifest_id,
                    },
                )?)?;
            }
            Step::ReadBatches => loop {
                let batch: EvidenceBatch = transport.read()?.context("core closed")?.params()?;
                items.extend(batch.items);
                if batch.last {
                    break;
                }
            },
            Step::StateClaims => {
                for item in items.iter().filter(|i| i.span.field == "/state") {
                    sent += 1;
                    transport.send(&Envelope::notification(
                        CLAIM_SUBMIT,
                        json!({
                            "subject": {"identifier": item.location, "type": "code:PullRequest"},
                            "predicate": "code:state",
                            "object": {"kind": "literal", "value": item.span.quote, "datatype": "string"},
                            "extraction_rule": "state-field",
                            "supports": [{"evidence_id": item.evidence_id, "field": "/state",
                                "start": item.span.start, "end": item.span.end}],
                        }),
                    )?)?;
                }
            }
            Step::Send { message } => {
                sent += 1;
                transport.send(&fill(message, &items))?;
            }
            Step::Finish { status } => {
                let finish = RunFinish {
                    status,
                    submitted: sent,
                    reason: None,
                };
                transport.send(&Envelope::request(1, RUN_FINISH, finish)?)?;
                if transport.read()?.is_none() {
                    bail!("core closed before answering run.finish");
                }
            }
            Step::Touch { path } => fs::write(path, "")?,
            Step::Gate {
                source: selected,
                entered,
                release,
                fail,
            } => {
                if source == selected {
                    fs::write(entered, "")?;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                    while !std::path::Path::new(&release).exists() {
                        if std::time::Instant::now() >= deadline {
                            bail!("gate timed out");
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    if fail {
                        bail!("scripted failure after staging an earlier source");
                    }
                }
            }
            Step::Drain => while transport.read()?.is_some() {},
            Step::Exit { code } => std::process::exit(code),
        }
    }
    Ok(())
}
