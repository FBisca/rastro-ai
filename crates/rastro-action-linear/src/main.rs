//! The Linear action provider: an `action_provider` plugin that creates the issue an accepted
//! proposal asked for (ADR 0015). It reads no evidence and holds the Linear key in its own
//! environment grant, which no other plugin is given.
//!
//! `action.execute` creates the issue with the run's random id as the issue's own, from the title,
//! description, team and assignee the owner accepted and nothing else: the why, the citations and
//! every Rastro id stay in Rastro. `action.lookup` finds the issue under that id, so a run
//! executed again, or whose answer was lost, learns what the first execution did before it creates
//! anything. `action.undo` deletes the issue under that id, unless Linear already holds none, so
//! an undo whose answer was lost deletes nothing the second time. Any other action message is
//! answered with an error.
mod linear;

use anyhow::{Context, Result, bail};
use linear::{Client, Issue};
use rastro_plugin_sdk::{
    ACTION_EXECUTE, ACTION_LOOKUP, ACTION_UNDO, ActionCall, ActionInitializeParams, ActionResult,
    Envelope, INITIALIZE, InitializeResult, PROTOCOL_VERSION, Transport,
};
use serde_json::json;
use std::io;

const LINEAR_CREATE_ISSUE: &str = "linear.create_issue";

fn main() -> Result<()> {
    let mut transport = Transport::new(io::stdin().lock(), io::stdout().lock());
    while let Some(request) = transport.read()? {
        let Some(id) = request.id.clone() else {
            continue;
        };
        let answer = match request.method.as_deref() {
            Some(INITIALIZE) => request
                .params::<ActionInitializeParams>()
                .and_then(|params| {
                    Envelope::response(
                        Some(id.clone()),
                        InitializeResult {
                            protocol: PROTOCOL_VERSION.into(),
                            manifest_id: params.manifest_id,
                        },
                    )
                }),
            Some(ACTION_EXECUTE) => request
                .params::<ActionCall>()
                .and_then(|call| execute(&call))
                .and_then(|result| Envelope::response(Some(id.clone()), result)),
            Some(ACTION_LOOKUP) => request
                .params::<ActionCall>()
                .and_then(|call| lookup(&call))
                .and_then(|result| Envelope::response(Some(id.clone()), result)),
            Some(ACTION_UNDO) => request
                .params::<ActionCall>()
                .and_then(|call| undo(&call))
                .and_then(|result| Envelope::response(Some(id.clone()), result)),
            other => Err(anyhow::anyhow!(
                "this provider does not handle {}",
                other.unwrap_or("that message")
            )),
        };
        transport.send(&answer.unwrap_or_else(|error| failure(id, &error)))?;
    }
    Ok(())
}

/// A JSON-RPC error response carrying `error`'s message.
fn failure(id: serde_json::Value, error: &anyhow::Error) -> Envelope {
    Envelope {
        jsonrpc: "2.0".into(),
        id: Some(id),
        error: Some(json!({"code": -32000, "message": format!("{error:#}")})),
        ..Envelope::default()
    }
}

/// Finds the issue the call's key names: `found` is false when Linear holds none.
fn lookup(call: &ActionCall) -> Result<ActionResult> {
    if call.action != LINEAR_CREATE_ISSUE {
        bail!("this provider does not offer {}", call.action);
    }
    Ok(match Client::from_env()?.find_issue(&call.key)? {
        Some(issue) => ActionResult {
            found: true,
            identifier: Some(issue.identifier),
            url: Some(issue.url),
        },
        None => ActionResult::default(),
    })
}

/// Deletes the issue the call's key names: `found` is true when it deleted one, and false when
/// Linear already held none, so a repeated undo sends no second delete.
fn undo(call: &ActionCall) -> Result<ActionResult> {
    if call.action != LINEAR_CREATE_ISSUE {
        bail!("this provider does not offer {}", call.action);
    }
    Ok(ActionResult {
        found: Client::from_env()?.delete_issue(&call.key)?,
        ..ActionResult::default()
    })
}

/// Creates the issue the call names.
fn execute(call: &ActionCall) -> Result<ActionResult> {
    if call.action != LINEAR_CREATE_ISSUE {
        bail!("this provider does not offer {}", call.action);
    }
    let field = |name: &str| {
        (call.fields.get(name))
            .map(String::as_str)
            .with_context(|| format!("the call has no {name}"))
    };
    let created = Client::from_env()?.create_issue(&Issue {
        id: &call.key,
        title: field("title")?,
        description: call.fields.get("description").map(String::as_str),
        team: field("team")?,
        assignee: field("assignee")?,
    })?;
    Ok(ActionResult {
        found: true,
        identifier: Some(created.identifier),
        url: Some(created.url),
    })
}
