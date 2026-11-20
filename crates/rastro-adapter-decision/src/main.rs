//! The decision adapter: it lets the owner's scoring service (jev) score text (ADR-0016 §5,
//! decisions/plugins.md §41).
//!
//! The core sends `initialize` and then `decision.request`s. This adapter POSTs the caller's
//! instructions, input and schema to the endpoint `RASTRO_DECISION_ENDPOINT` names and relays
//! the JSON it answers with, unchanged. It checks nothing beyond "a JSON value": the output's
//! shape is the caller's schema's business, and the core validates it and keeps only what passes
//! (ADR-0016 §2). It decides nothing and writes nothing.
//!
//! `RASTRO_DECISION_ENDPOINT` is the endpoint's whole URL, path included; it reaches the adapter
//! only because the manifest lists it. The live jev contract is verified when binding lands;
//! until then the crate is proven against a fake scoring service.
use anyhow::{Context, Result, bail};
use rastro_plugin_sdk::{
    AdapterInitializeParams, DECISION_REQUEST, DecisionRequest, DecisionResponse, Envelope,
    INITIALIZE, InitializeResult, PROTOCOL_VERSION, Transport,
};
use serde_json::{Value, json};
use std::{io, time::Duration};

/// The manifest's `id`: what the core checks in the answer to `initialize`.
const MANIFEST_ID: &str = "rastro-adapter-decision";

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut wire = Transport::new(stdin.lock(), io::stdout().lock());
    while let Some(message) = wire.read()? {
        match message.method.as_deref() {
            Some(INITIALIZE) => {
                let init: AdapterInitializeParams = message.params()?;
                if init.manifest_id != MANIFEST_ID {
                    bail!("the core started {} as {MANIFEST_ID}", init.manifest_id);
                }
                wire.send(&Envelope::response(
                    message.id,
                    InitializeResult {
                        protocol: PROTOCOL_VERSION.into(),
                        manifest_id: MANIFEST_ID.into(),
                    },
                )?)?;
            }
            Some(DECISION_REQUEST) => {
                let request: DecisionRequest = message.params()?;
                let reply = match score(&request) {
                    Ok(output) => Envelope::response(message.id, DecisionResponse { output })?,
                    Err(error) => Envelope {
                        jsonrpc: "2.0".into(),
                        id: message.id,
                        error: Some(json!({"code": -32000, "message": format!("{error:#}")})),
                        ..Envelope::default()
                    },
                };
                wire.send(&reply)?;
            }
            _ => bail!("unexpected message {:?}", message.method),
        }
    }
    Ok(())
}

/// POSTs the caller's instructions, input and schema to the configured scoring endpoint and
/// relays its JSON answer. A missing endpoint, an unreachable one, an HTTP failure or a non-JSON
/// answer is an error: the run fails and no partial output escapes (ADR-0016 §8).
fn score(request: &DecisionRequest) -> Result<Value> {
    let endpoint = std::env::var("RASTRO_DECISION_ENDPOINT")
        .ok()
        .filter(|url| !url.is_empty())
        .context("RASTRO_DECISION_ENDPOINT must name the scoring endpoint's URL")?;
    // No redirects: the input follows no answer to another host. The HTTP call gets the core's
    // per-turn budget (decisions/plugins.md §45); the core's cap still governs the turn.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .into();
    let body = json!({
        "instructions": request.instructions,
        "input": request.input,
        "schema": request.schema,
    });
    let mut response = agent
        .post(&endpoint)
        .send_json(&body)
        .with_context(|| format!("can't reach the scoring endpoint {endpoint}"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("the scoring endpoint answered HTTP {status}");
    }
    let text = response
        .body_mut()
        .read_to_string()
        .context("reading the scoring endpoint's answer")?;
    // Any JSON value, unchanged: the caller's schema may ask for any shape, and the core's gate
    // alone decides what is kept. Repairing or refusing a broken shape here would usurp it.
    serde_json::from_str(&text).context("the scoring endpoint's answer is not JSON")
}
