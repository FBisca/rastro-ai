//! A connector driven by a script, so tests can make it behave well or badly.
//!
//! `RASTRO_TEST_SCRIPT` names a JSONL file; each line is one step, tagged by `do`:
//! - `initialize` {protocol?, manifest_id?}: read the core's `initialize` and answer it
//! - `send` {message}: write that JSON object as one line
//! - `send_raw` {line}: write that text as one line (malformed input)
//! - `finish` {status?, submitted?, reason?}: send `run.finish` and wait for the reply
//! - `sleep` {ms}
//! - `dump_env` {path}: write the environment, one NAME=value per line, to that file
//! - `exit` {code?}: end the process at once
//!
//! Without a script it initializes, reports complete coverage and finishes.
use anyhow::{Context, Result, bail};
use rastro_plugin_sdk::{
    Envelope, InitializeParams, InitializeResult, PROTOCOL_VERSION, RUN_FINISH, RunFinish,
    Transport,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{env, fs, io, thread, time::Duration};

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Step {
    Initialize {
        protocol: Option<String>,
        manifest_id: Option<String>,
    },
    Send {
        message: Value,
    },
    SendRaw {
        line: String,
    },
    Finish {
        #[serde(default = "ok")]
        status: String,
        #[serde(default)]
        submitted: u64,
        reason: Option<String>,
    },
    Sleep {
        ms: u64,
    },
    DumpEnv {
        path: String,
    },
    Exit {
        #[serde(default)]
        code: i32,
    },
}
fn ok() -> String {
    "ok".into()
}

fn script() -> Result<Vec<Step>> {
    let Some(path) = env::var_os("RASTRO_TEST_SCRIPT").filter(|p| fs::metadata(p).is_ok()) else {
        return Ok(serde_json::from_value(json!([
            {"do": "initialize"},
            {"do": "send", "message": {"jsonrpc": "2.0", "method": "coverage.report",
                "params": {"status": "complete"}}},
            {"do": "finish"}
        ]))?);
    };
    fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).context("invalid script step"))
        .collect()
}

fn main() -> Result<()> {
    let mut transport = Transport::new(io::stdin().lock(), io::stdout().lock());
    let mut next_id = 100;
    for step in script()? {
        match step {
            Step::Initialize {
                protocol,
                manifest_id,
            } => {
                let request = transport.read()?.context("core closed before initialize")?;
                let params: InitializeParams = request.params()?;
                transport.send(&Envelope::response(
                    request.id,
                    InitializeResult {
                        protocol: protocol.unwrap_or_else(|| PROTOCOL_VERSION.into()),
                        manifest_id: manifest_id.unwrap_or(params.manifest_id),
                    },
                )?)?;
            }
            Step::Send { message } => transport.send(&message)?,
            Step::SendRaw { line } => transport.send_line(&line)?,
            Step::Finish {
                status,
                submitted,
                reason,
            } => {
                next_id += 1;
                let finish = RunFinish {
                    status,
                    submitted,
                    reason,
                };
                transport.send(&Envelope::request(next_id, RUN_FINISH, finish)?)?;
                if transport.read()?.is_none() {
                    bail!("core closed before answering run.finish");
                }
            }
            Step::Sleep { ms } => thread::sleep(Duration::from_millis(ms)),
            Step::DumpEnv { path } => {
                let lines: Vec<_> = env::vars().map(|(k, v)| format!("{k}={v}")).collect();
                fs::write(path, lines.join("\n"))?;
            }
            Step::Exit { code } => std::process::exit(code),
        }
    }
    Ok(())
}
