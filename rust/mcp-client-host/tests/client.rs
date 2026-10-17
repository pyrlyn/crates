//! The client over an in-process duplex: it namespaces a server's tools,
//! survives a server that dies mid-session, turns a server that never starts
//! into a notice, answers a server's `elicitation/create` through the asker
//! (declining when nobody can answer), and ignores a tool's MCP App UI
//! resource: the handshake declares no `io.modelcontextprotocol/ui`
//! extension, a `ui://` resource is never fetched, and a tool with
//! `_meta.ui.resourceUri` yields the same output as its UI-less twin.

// why: helpers outside `#[test]` fns still assert with expect/panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mcp_client_host::client::{Host, McpClient, connect_all};
use mcp_client_host::config::ServerConfig as Entry;
use mcp_client_host::elicit::{Ask, Asker};
use mcp_client_host::tool::{CallError, Risk};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientResult,
    ContentBlock, ElicitRequest, ElicitRequestParams, ListToolsResult, MetaObject,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    ResourceContents, ServerCapabilities, ServerConfig, ServerRequest,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A server with one tool, `echo`, that answers `echo: <text>`.
#[derive(Clone)]
struct Echo;

impl ServerHandler for Echo {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let schema = json!({"type":"object","properties":{"text":{"type":"string"}}});
        let schema = schema.as_object().expect("object").clone();
        Ok(ListToolsResult::with_all_items(vec![
            rmcp::model::Tool::new("echo", "echo", schema),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let text = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(format!("echo: {text}"))]).into())
    }
}

/// An `Echo` on one end of a duplex; returns the client end and the server
/// task so a test can kill the server.
async fn serve() -> (McpClient, tokio::task::JoinHandle<()>) {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(async move {
        use rmcp::ServiceExt;
        let running = Echo
            .serve(tokio::io::split(server_io))
            .await
            .expect("server handshake");
        let _ = running.waiting().await;
    });
    let client = McpClient::from_transport(
        "t",
        tokio::io::split(client_io),
        Duration::from_secs(2),
        None,
    )
    .await
    .expect("client handshake");
    (client, task)
}

#[tokio::test]
async fn client_round_trips_a_namespaced_deferred_call_over_a_duplex() {
    let (client, task) = serve().await;
    let tools = client.tools(true).await.expect("list");
    assert_eq!(tools.len(), 1);
    let spec = tools[0].spec();
    assert_eq!(spec.name, "mcp__t__echo");
    assert!(spec.deferred);
    // No annotations from the server: the default risk is `Write`.
    assert_eq!(spec.risk, Risk::Write);
    assert_eq!(spec.input_schema["type"], "object");
    let out = tools[0]
        .call(json!({ "text": "hi" }), &CancellationToken::new())
        .await
        .expect("call");
    assert!(!out.is_error);
    assert_eq!(out.text, "echo: hi");
    client.close().await;
    task.abort();
}

#[tokio::test]
async fn a_cancelled_token_ends_the_call_as_cancelled() {
    let (client, task) = serve().await;
    let tools = client.tools(true).await.expect("list");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = tools[0].call(json!({}), &cancel).await;
    assert_eq!(result, Err(CallError::Cancelled));
    task.abort();
}

#[tokio::test]
async fn server_crash_does_not_end_session_and_a_server_that_never_starts_is_a_notice() {
    let (client, task) = serve().await;
    let tools = client.tools(true).await.expect("list");
    task.abort();
    let _ = task.await;
    // The dead server is an error the model reads, not a panic or a hang.
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tools[0].call(json!({ "text": "hi" }), &CancellationToken::new()),
    )
    .await
    .expect("no hang");
    match result {
        Ok(out) => assert!(out.is_error, "{}", out.text),
        Err(e) => assert_eq!(e, CallError::Timeout),
    }
    // A server that never starts is a notice, not an error.
    let mut servers = HashMap::new();
    servers.insert(
        "ghost".to_string(),
        Entry {
            command: Some("/definitely/not/a/server".into()),
            ..Entry::default()
        },
    );
    let (clients, tools, notices) =
        connect_all(&servers, Duration::from_secs(2), true, &Host::none()).await;
    assert!(clients.is_empty() && tools.is_empty());
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].starts_with("mcp server `ghost` skipped:"),
        "{notices:?}"
    );
}

#[tokio::test]
async fn one_failing_server_does_not_stop_the_others() {
    let servers = HashMap::from([
        (
            "a-ghost".to_string(),
            Entry {
                command: Some("/no/such/server".into()),
                ..Entry::default()
            },
        ),
        ("b-empty".to_string(), Entry::default()),
    ]);
    let (clients, tools, notices) =
        connect_all(&servers, Duration::from_secs(2), true, &Host::none()).await;
    assert!(clients.is_empty() && tools.is_empty());
    assert_eq!(notices.len(), 2);
    assert!(
        notices[0].starts_with("mcp server `a-ghost` skipped:"),
        "{notices:?}"
    );
    assert_eq!(
        notices[1],
        "mcp server `b-empty` skipped: server `b-empty` has neither `command` nor `url`"
    );
}

/// An MCP server whose one tool, `ask`, sends `params` as an
/// `elicitation/create` and returns the client's answer as JSON text. It
/// records the capabilities the client declared at the handshake.
#[derive(Clone)]
struct Eliciting {
    params: ElicitRequestParams,
    caps: Arc<Mutex<Option<ClientCapabilities>>>,
}

impl ServerHandler for Eliciting {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![
            rmcp::model::Tool::new("ask", "asks the person", serde_json::Map::new()),
        ]))
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if let Ok(mut caps) = self.caps.lock() {
            *caps = context.peer.peer_info().map(|i| i.capabilities.clone());
        }
        let sent = context
            .peer
            .send_request(ServerRequest::ElicitRequest(ElicitRequest::new(
                self.params.clone(),
            )))
            .await;
        let text = match sent {
            Ok(ClientResult::ElicitResult(result)) => {
                serde_json::to_string(&result).expect("elicit result json")
            }
            other => format!("unexpected: {other:?}"),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
    }
}

/// A two-field form; `from_str` keeps `name` before `age`.
fn form() -> ElicitRequestParams {
    serde_json::from_str(
        r#"{"mode":"form","message":"Sign up","requestedSchema":{"type":"object",
            "properties":{"name":{"type":"string"},"age":{"type":"integer","minimum":0}},
            "required":["name"]}}"#,
    )
    .expect("form params")
}

/// The person at the surface: answers each question in turn after
/// `delay`; `None` dismisses it (Esc). Returns the questions it saw.
fn person(
    answers: Vec<Option<&'static str>>,
    delay: Duration,
) -> (Asker, tokio::task::JoinHandle<Vec<String>>) {
    let (asker, mut asks) = mpsc::channel::<Ask>(4);
    let task = tokio::spawn(async move {
        let mut seen = Vec::new();
        for answer in answers {
            let Some(ask) = asks.recv().await else { break };
            assert_eq!(ask.server, "e");
            seen.push(ask.question);
            tokio::time::sleep(delay).await;
            if let Some(text) = answer {
                let _ = ask.reply.send(text.to_string());
            }
        }
        seen
    });
    (asker, task)
}

/// Calls `ask` on an [`Eliciting`] server; returns the answer the server
/// got and the capabilities it saw.
async fn elicit(
    params: ElicitRequestParams,
    ask: Option<Asker>,
    timeout: Duration,
) -> (Result<Value, CallError>, Option<ClientCapabilities>) {
    let caps = Arc::new(Mutex::new(None));
    let server = Eliciting {
        params,
        caps: caps.clone(),
    };
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(async move {
        use rmcp::ServiceExt;
        let running = server
            .serve(tokio::io::split(server_io))
            .await
            .expect("server handshake");
        let _ = running.waiting().await;
    });
    let client = McpClient::from_transport("e", tokio::io::split(client_io), timeout, ask)
        .await
        .expect("client handshake");
    let tools = client.tools(false).await.expect("list");
    let out = tools[0]
        .call(json!({}), &CancellationToken::new())
        .await
        .map(|out| {
            assert!(!out.is_error, "{}", out.text);
            serde_json::from_str(&out.text).expect("answer json")
        });
    client.close().await;
    task.abort();
    let caps = caps.lock().expect("caps").clone();
    (out, caps)
}

#[tokio::test]
async fn form_elicitation_round_trips_through_the_asker() {
    let (asker, seen) = person(vec![Some("Ada"), Some("36"), Some("send")], Duration::ZERO);
    let (answer, caps) = elicit(form(), Some(asker), Duration::from_secs(5)).await;
    assert_eq!(
        answer.expect("call"),
        json!({ "action": "accept", "content": { "name": "Ada", "age": 36 } })
    );
    let caps = caps.expect("client capabilities");
    let elicitation = caps.elicitation.expect("elicitation");
    assert!(elicitation.form.is_some());
    assert!(elicitation.url.is_some());
    let seen = seen.await.expect("person");
    assert_eq!(seen.len(), 3);
    assert!(seen[0].starts_with("Sign up — name"), "{seen:?}");
    // The review names the fields, never the answers (A78).
    assert_eq!(seen[2], "Sign up — send name, age?");
}

#[tokio::test]
async fn review_decline_answers_decline() {
    let (asker, _seen) = person(vec![Some("Ada"), Some(""), Some("decline")], Duration::ZERO);
    let (answer, _) = elicit(form(), Some(asker), Duration::from_secs(5)).await;
    assert_eq!(answer.expect("call"), json!({ "action": "decline" }));
}

#[tokio::test]
async fn dismissed_question_answers_cancel() {
    let (asker, _seen) = person(vec![None], Duration::ZERO);
    let (answer, _) = elicit(form(), Some(asker), Duration::from_secs(5)).await;
    assert_eq!(answer.expect("call"), json!({ "action": "cancel" }));
}

#[tokio::test]
async fn no_asker_declares_no_elicitation_capability() {
    let (answer, caps) = elicit(form(), None, Duration::from_secs(5)).await;
    assert!(caps.expect("client capabilities").elicitation.is_none());
    assert_eq!(answer.expect("call"), json!({ "action": "decline" }));
}

/// A78: the call's 200 ms deadline stops while the person takes 500 ms
/// per question.
#[tokio::test]
async fn waiting_for_a_person_does_not_time_out_the_call() {
    let (asker, _seen) = person(
        vec![Some("Ada"), Some("7"), Some("send")],
        Duration::from_millis(500),
    );
    let (answer, _) = elicit(form(), Some(asker), Duration::from_millis(200)).await;
    assert_eq!(
        answer.expect("no timeout while asking"),
        json!({ "action": "accept", "content": { "name": "Ada", "age": 7 } })
    );
}

/// A URL elicitation as a server sends it.
fn url(target: &str) -> ElicitRequestParams {
    serde_json::from_value(json!({
        "mode": "url",
        "message": "Link your account",
        "url": target,
        "elicitationId": "e1",
    }))
    .expect("url params")
}

#[tokio::test]
async fn url_elicitation_shows_the_url_and_declines_on_decline() {
    // Only `decline` is answered here: `open` would launch a real browser.
    let (asker, seen) = person(vec![Some("decline")], Duration::ZERO);
    let (answer, _) = elicit(
        url("https://example.com/cb"),
        Some(asker),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(answer.expect("call"), json!({ "action": "decline" }));
    let seen = seen.await.expect("person");
    assert_eq!(
        seen,
        ["Link your account · open https://example.com/cb · host: example.com"]
    );
}

#[tokio::test]
async fn file_scheme_elicitation_is_declined_unasked() {
    let (asker, seen) = person(vec![], Duration::ZERO);
    let (answer, _) = elicit(
        url("file:///etc/passwd"),
        Some(asker),
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(answer.expect("call"), json!({ "action": "decline" }));
    assert!(seen.await.expect("person").is_empty());
}

/// T55.1: a server with two tools, `app` (carrying `_meta.ui.resourceUri`,
/// per the MCP Apps extension) and `plain` (the same tool without it), both
/// returning identical text and structured content. Advertises the
/// `resources` capability so a client that wanted the UI resource could
/// fetch it; records whether it ever did, and the capabilities the client
/// declared at the handshake.
#[derive(Clone, Default)]
struct UiServer {
    caps: Arc<Mutex<Option<ClientCapabilities>>>,
    resource_reads: Arc<AtomicUsize>,
}

impl ServerHandler for UiServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if let Ok(mut caps) = self.caps.lock() {
            *caps = context.peer.peer_info().map(|i| i.capabilities.clone());
        }
        let ui_meta = MetaObject(
            json!({ "ui": { "resourceUri": "ui://widget" } })
                .as_object()
                .expect("object")
                .clone(),
        );
        Ok(ListToolsResult::with_all_items(vec![
            rmcp::model::Tool::new("app", "renders a widget", serde_json::Map::new())
                .with_meta(ui_meta),
            rmcp::model::Tool::new("plain", "renders a widget", serde_json::Map::new()),
        ]))
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let mut result = CallToolResult::success(vec![ContentBlock::text("42")]);
        result.structured_content = Some(json!({ "value": 42 }));
        Ok(result.into())
    }

    async fn read_resource(
        &self,
        _request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        self.resource_reads.fetch_add(1, Ordering::SeqCst);
        Ok(
            ReadResourceResult::new(vec![ResourceContents::text("<html></html>", "ui://widget")])
                .into(),
        )
    }
}

/// A [`UiServer`] on one end of a duplex; returns the client, the server
/// task, the resource-read counter and the capabilities the handshake
/// recorded.
async fn ui_server() -> (
    McpClient,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    Arc<Mutex<Option<ClientCapabilities>>>,
) {
    let server = UiServer::default();
    let resource_reads = server.resource_reads.clone();
    let caps = server.caps.clone();
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let task = tokio::spawn(async move {
        use rmcp::ServiceExt;
        let running = server
            .serve(tokio::io::split(server_io))
            .await
            .expect("server handshake");
        let _ = running.waiting().await;
    });
    let client = McpClient::from_transport(
        "ui",
        tokio::io::split(client_io),
        Duration::from_secs(2),
        None,
    )
    .await
    .expect("client handshake");
    (client, task, resource_reads, caps)
}

#[tokio::test]
async fn mcp_app_tool_output_matches_the_same_tool_without_ui() {
    let (client, task, _reads, _caps) = ui_server().await;
    let tools = client.tools(false).await.expect("list");
    let app = tools
        .iter()
        .find(|t| t.qualified_name() == "mcp__ui__app")
        .expect("app tool");
    let plain = tools
        .iter()
        .find(|t| t.qualified_name() == "mcp__ui__plain")
        .expect("plain tool");
    let app_out = app
        .call(json!({}), &CancellationToken::new())
        .await
        .expect("app call");
    let plain_out = plain
        .call(json!({}), &CancellationToken::new())
        .await
        .expect("plain call");
    assert!(!app_out.is_error, "{}", app_out.text);
    assert_eq!(app_out, plain_out);
    assert_eq!(app_out.structured, Some(json!({ "value": 42 })));
    client.close().await;
    task.abort();
}

#[tokio::test]
async fn mcp_client_declares_no_ui_extension() {
    let (client, task, _reads, caps) = ui_server().await;
    let _ = client.tools(false).await.expect("list");
    let caps = caps
        .lock()
        .expect("caps")
        .clone()
        .expect("client capabilities");
    assert!(caps.extensions.is_none(), "{:?}", caps.extensions);
    client.close().await;
    task.abort();
}

#[tokio::test]
async fn mcp_client_never_reads_a_ui_resource() {
    let (client, task, reads, _caps) = ui_server().await;
    let tools = client.tools(false).await.expect("list");
    for tool in &tools {
        let out = tool
            .call(json!({}), &CancellationToken::new())
            .await
            .expect("call");
        assert!(!out.is_error, "{}", out.text);
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    client.close().await;
    task.abort();
}
