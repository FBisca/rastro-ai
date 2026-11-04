//! `rastro mcp`: the MCP server (ADR 0006). It speaks MCP revision 2025-11-25 over stdio,
//! offers the catalog's MCP operations, and opens the store read-only for every call. tokio and
//! rmcp live in this module only; the core stays synchronous and runs through `spawn_blocking`.
use anyhow::{Context, Result};
use rastro_core::{Locations, Rastro};
use rastro_operations::{
    catalog::{self, OperationSpec, Surface},
    envelope,
};
use rastro_storage_sqlite::SqliteStorage;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::{RequestContext, ServerInitializeError},
};
use serde_json::Value;
use std::{borrow::Cow, time::Duration};

/// The one protocol revision this server speaks.
const PINNED: ProtocolVersion = ProtocolVersion::V_2025_11_25;
/// A diagnostic on stderr is cut to this many bytes.
const STDERR_LIMIT: usize = 2048;
/// How long the runtime may take to wind down once the client has gone.
const SHUTDOWN: Duration = Duration::from_secs(5);

fn specs() -> impl Iterator<Item = &'static OperationSpec> {
    catalog::CATALOG
        .iter()
        .filter(|spec| spec.exposed_on(Surface::Mcp))
}

fn tool(spec: &OperationSpec) -> Tool {
    let Value::Object(schema) = spec.input_schema() else {
        unreachable!("an input schema is an object")
    };
    Tool::new(spec.name, spec.description(), schema).with_annotations(
        ToolAnnotations::new()
            .read_only(spec.read_only)
            .open_world(false),
    )
}

/// The server: it holds nothing. Each call opens the store itself, and what a client says about
/// itself is never read.
struct Server;

/// What one tool call does, off the async runtime: open read-only, run, wrap the envelope.
fn run_operation(
    spec: &'static OperationSpec,
    arguments: Value,
) -> Result<CallToolResult, ErrorData> {
    let request = spec
        .decode(arguments)
        .map_err(|e| ErrorData::invalid_params(format!("invalid arguments: {e}"), None))?;
    let outcome = Locations::resolve().and_then(|locations| {
        let mut reader = Rastro::open_read_only(locations, SqliteStorage::open_read_only)?;
        request.run_read(&mut reader)
    });
    Ok(match outcome {
        Ok(response) => CallToolResult::structured(response.envelope()),
        Err(error) => {
            CallToolResult::structured_error(envelope::failure(spec.name, format!("{error:#}")))
        }
    })
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(PINNED)
            .with_server_info(Implementation::new("rastro", env!("CARGO_PKG_VERSION")))
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![PINNED])
    }
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(specs().map(tool).collect()))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let spec = specs()
            .find(|spec| spec.name == request.name)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("unknown tool: {}", request.name), None)
            })?;
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = tokio::task::spawn_blocking(move || run_operation(spec, arguments))
            .await
            .map_err(|e| ErrorData::internal_error(format!("the call failed: {e}"), None))??;
        Ok(result.into())
    }
}

/// A message for stderr, cut to its bound on a character boundary.
pub fn bounded(message: &str) -> &str {
    if message.len() <= STDERR_LIMIT {
        return message;
    }
    let mut end = STDERR_LIMIT;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    &message[..end]
}

/// Runs the server until the client closes stdin.
pub fn run() -> Result<()> {
    // The normal writing open, once: migrations, source projection, recovery, WAL. An unknown
    // newer migration fails here, before a byte is written to stdout.
    {
        let locations = Locations::resolve()?;
        let storage = SqliteStorage::open(&locations.database())?;
        Rastro::open(locations, storage)?;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let served = runtime.block_on(async {
        match Server.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                running.waiting().await.context("serving")?;
                Ok(())
            }
            // A client that leaves before it initializes is not an error.
            Err(ServerInitializeError::ConnectionClosed(_)) => Ok(()),
            Err(error) => Err(anyhow::Error::from(error)),
        }
    });
    runtime.shutdown_timeout(SHUTDOWN);
    served
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn S23_bounded_cuts_multi_byte_text_to_the_limit_on_a_character_boundary() {
        // 3-byte characters, with a prefix that puts the limit inside one.
        let message = format!("e{}", "€".repeat(2000));
        let cut = bounded(&message);
        assert!(cut.len() <= STDERR_LIMIT && cut.len() > STDERR_LIMIT - 4);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
        assert_eq!(bounded("short"), "short");
    }
}
