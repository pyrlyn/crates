//! One server tool as the host sees it: a neutral spec the host maps onto
//! its own tool type, and a call that is cancellable, bounded by the
//! client's timeout (which pauses while a person answers a question) and
//! never panics. A failed call is an error result the model can read.

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::asking::within;
use crate::client::McpClient;

/// How risky a call is. A server's annotations are hints from an untrusted
/// peer, so only `readOnlyHint` lowers the risk, `destructiveHint` raises it
/// and silence means `Write`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// The server says the tool changes nothing.
    ReadOnly,
    /// The default: it may change something.
    Write,
    /// The server says the tool can destroy data.
    Destructive,
}

/// Whether calls to a tool may run alongside others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Concurrency {
    /// Read-only tools may run in parallel.
    Parallel,
    /// Everything else runs alone.
    Exclusive,
}

/// A tool's advertised shape.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// `mcp__<server>__<tool>`.
    pub name: String,
    /// Shown to the model; empty when the server gave none.
    pub description: String,
    /// JSON Schema for the tool's input.
    pub input_schema: Value,
    /// True when the host should list the tool only through search.
    pub deferred: bool,
    /// See [`Risk`].
    pub risk: Risk,
    /// See [`Concurrency`].
    pub concurrency: Concurrency,
}

/// A tool call's result.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// The text content blocks, joined by newlines.
    pub text: String,
    /// Whether the server (or the transport) reported a failure.
    pub is_error: bool,
    /// The server's `structuredContent`, when it sent one.
    pub structured: Option<Value>,
}

/// Why a call produced no result at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// The caller's token fired.
    #[error("cancelled")]
    Cancelled,
    /// The server did not answer within the client's timeout.
    #[error("timed out")]
    Timeout,
}

/// One tool of a connected server. Cloning shares the session.
#[derive(Clone)]
pub struct McpTool {
    pub(crate) client: McpClient,
    pub(crate) tool: rmcp::model::Tool,
    pub(crate) deferred: bool,
}

impl McpTool {
    /// `mcp__<server>__<tool>`: the name the model calls it by.
    pub fn qualified_name(&self) -> String {
        format!("mcp__{}__{}", self.client.name(), self.tool.name)
    }

    /// The tool's shape; the host maps it onto its own tool type.
    pub fn spec(&self) -> ToolSpec {
        let ann = self.tool.annotations.as_ref();
        let risk = match (
            ann.and_then(|a| a.read_only_hint),
            ann.and_then(|a| a.destructive_hint),
        ) {
            (Some(true), _) => Risk::ReadOnly,
            (_, Some(true)) => Risk::Destructive,
            _ => Risk::Write,
        };
        ToolSpec {
            name: self.qualified_name(),
            description: self
                .tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_string(),
            input_schema: Value::Object((*self.tool.input_schema).clone()),
            deferred: self.deferred,
            risk,
            concurrency: if risk == Risk::ReadOnly {
                Concurrency::Parallel
            } else {
                Concurrency::Exclusive
            },
        }
    }

    /// Calls the tool. A server error is `Ok` with `is_error` set so the
    /// model can read it; only a cancellation or a timeout is `Err`.
    pub async fn call(
        &self,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<ToolOutput, CallError> {
        let mut params = CallToolRequestParams::new(self.tool.name.clone());
        if let Some(args) = input.as_object() {
            params = params.with_arguments(args.clone());
        }
        let call = self.client.service.call_tool(params);
        let asking = &self.client.asking;
        let result = tokio::select! {
            () = cancel.cancelled() => {
                // A question this server still has open would otherwise
                // wait for an answer nobody needs.
                asking.abort_all();
                return Err(CallError::Cancelled);
            }
            r = within(asking, self.client.timeout, call) => match r {
                Some(Ok(result)) => result,
                Some(Err(e)) => return Ok(ToolOutput {
                    text: format!("mcp server `{}`: {e}", self.client.name()),
                    is_error: true,
                    structured: None,
                }),
                None => return Err(CallError::Timeout),
            },
        };
        Ok(output_of(result))
    }
}

fn output_of(result: CallToolResult) -> ToolOutput {
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    ToolOutput {
        text,
        is_error: result.is_error.unwrap_or(false),
        structured: result.structured_content,
    }
}
