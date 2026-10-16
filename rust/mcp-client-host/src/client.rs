//! The MCP client: one rmcp session per server, its tools exposed as
//! [`McpTool`]s named `mcp__<server>__<tool>`. A server that will not start
//! is a notice and no tools (fail open); a call that fails is an error
//! result the model can read, never a crash.
//! HTTP servers go through rmcp's `AuthClient`: a stored token is attached
//! and refreshed silently; a 401 on the handshake asks for a login when the
//! host can run one, and is a notice naming the host's login command
//! otherwise — never a silent skip.
//! [`McpHandler`] is the handler every session serves: with a person present
//! it answers a server's `elicitation/create` through the host's asker, and
//! without one it declares nothing and declines.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    ClientConfig, ElicitRequestParams, ElicitResult, ElicitationAction, ElicitationCapability,
    ErrorData, FormElicitationCapability, UrlElicitationCapability,
};
use rmcp::service::{ClientInitializeError, RequestContext, RoleClient, RunningService};
use rmcp::transport::auth::{AuthClient, AuthError, CredentialStore, InMemoryCredentialStore};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::transport::{IntoTransport, TokioChildProcess};
use rmcp::{ClientHandler, RmcpError, ServiceExt};

use crate::asking::Asking;
use crate::auth::{self, Secrets};
use crate::config::ServerConfig;
use crate::elicit::{self, Asker, Opener};
use crate::tool::McpTool;

/// The environment a stdio server inherits unless the host says otherwise;
/// everything else (API keys above all) stays behind.
pub const DEFAULT_CHILD_ENV: &[&str] = &[
    "PATH", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR", "USER", "SHELL",
];

/// Why a server could not be connected.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The entry has neither a command nor a URL.
    #[error("server `{name}` has neither `command` nor `url`")]
    NoTransport { name: String },
    /// The stdio command would not start.
    #[error("spawn {command}: {source}")]
    Spawn {
        command: String,
        source: std::io::Error,
    },
    /// The MCP handshake failed.
    #[error("handshake: {0}")]
    Handshake(Box<RmcpError>),
    /// The server connected but would not list its tools.
    #[error("tools/list: {0}")]
    List(rmcp::ServiceError),
    /// The server wants a token this host cannot obtain right now.
    #[error("{} run `{login_command} {name}`", if *.expired { "token expired," } else { "login required," })]
    LoginRequired {
        name: String,
        expired: bool,
        login_command: String,
    },
    /// The login itself failed.
    #[error("login: {0}")]
    Auth(#[from] AuthError),
}

/// How a host hands the person the login URL to open.
pub type Prompt = Arc<dyn Fn(&str) + Send + Sync>;

/// What a host brings: where tokens live, how to spawn a stdio server and,
/// when a person is present, how to reach them — `prompt` hands them a login
/// URL, `ask` puts a server's elicitation questions to them. `None` for
/// either means nobody is there: a 401 is a notice, an elicitation is
/// declined.
#[derive(Clone)]
pub struct Host {
    /// Where OAuth tokens live.
    pub secrets: Arc<dyn Secrets>,
    /// Shows the person a login URL.
    pub prompt: Option<Prompt>,
    /// Puts a server's questions to the person.
    pub ask: Option<Asker>,
    /// Opens a URL a person consented to; defaults to [`auth::open_browser`].
    pub open: Opener,
    /// How the host calls itself to an authorization server.
    pub app_name: String,
    /// The command a person runs to log in, named in the notice for a server
    /// that needs a login nobody can run now: ``run `<login_command> <server>` ``.
    pub login_command: String,
    /// The environment variables a stdio server inherits; the rest is cleared.
    pub child_env: Vec<String>,
}

impl Host {
    /// No persistence, no prompt, no asker: stdio-only callers and tests.
    pub fn none() -> Self {
        Self {
            secrets: Arc::new(auth::Memory::default()),
            prompt: None,
            ask: None,
            open: Arc::new(auth::open_browser),
            app_name: "mcp-client-host".to_string(),
            login_command: "login".to_string(),
            child_env: DEFAULT_CHILD_ENV.iter().map(ToString::to_string).collect(),
        }
    }
}

/// The client side of every session. Declares `elicitation.form` and `.url`
/// only when `ask` is set, and answers through it; any elicitation without
/// an asker is declined.
#[derive(Clone)]
pub struct McpHandler {
    server: String,
    ask: Option<Asker>,
    asking: Arc<Asking>,
    /// The one way a browser opens, after a person consented.
    open: Opener,
}

impl McpHandler {
    fn new(server: &str, ask: Option<Asker>, open: Opener) -> Self {
        Self {
            server: server.to_string(),
            ask,
            asking: Arc::default(),
            open,
        }
    }
}

impl ClientHandler for McpHandler {
    fn get_info(&self) -> ClientConfig {
        let mut info = ClientConfig::default();
        if self.ask.is_some() {
            info.capabilities.elicitation = Some(
                ElicitationCapability::new()
                    .with_form(FormElicitationCapability::new().with_schema_validation(true))
                    .with_url(UrlElicitationCapability::new()),
            );
        }
        info
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        let decline = ElicitResult::new(ElicitationAction::Decline);
        let Some(asker) = &self.ask else {
            return Ok(decline);
        };
        let _open = self.asking.enter();
        let abort = self.asking.abort_token();
        let answer = async {
            match request {
                ElicitRequestParams::FormElicitationParams {
                    message,
                    requested_schema,
                    ..
                } => elicit::run_form(asker, &self.server, &message, &requested_schema).await,
                ElicitRequestParams::UrlElicitationParams { message, url, .. } => {
                    elicit::run_url(asker, &self.server, &message, &url, &self.open).await
                }
                _ => decline,
            }
        };
        Ok(tokio::select! {
            result = answer => result,
            () = context.ct.cancelled() => ElicitResult::new(ElicitationAction::Cancel),
            () = abort.cancelled() => ElicitResult::new(ElicitationAction::Cancel),
        })
    }
}

/// A connected server. Cloning shares the session; `close` ends it.
#[derive(Clone)]
pub struct McpClient {
    name: String,
    pub(crate) service: Arc<RunningService<RoleClient, McpHandler>>,
    pub(crate) timeout: Duration,
    pub(crate) asking: Arc<Asking>,
}

impl McpClient {
    /// Spawns a stdio server or opens a Streamable HTTP one and handshakes.
    pub async fn connect(
        name: &str,
        cfg: &ServerConfig,
        timeout: Duration,
        host: &Host,
    ) -> Result<Self, ClientError> {
        let handler = McpHandler::new(name, host.ask.clone(), host.open.clone());
        match (&cfg.command, &cfg.url) {
            (Some(command), _) => {
                let mut cmd = tokio::process::Command::new(command);
                cmd.args(&cfg.args).env_clear();
                for key in &host.child_env {
                    if let Ok(v) = std::env::var(key) {
                        cmd.env(key, v);
                    }
                }
                cmd.envs(&cfg.env);
                let transport =
                    TokioChildProcess::new(cmd).map_err(|source| ClientError::Spawn {
                        command: command.clone(),
                        source,
                    })?;
                Self::serve(name, handler, transport, timeout).await
            }
            (None, Some(url)) => {
                let store = usable(host.secrets.store(name)).await;
                match Self::connect_http(name, url, store.clone(), timeout, handler.clone()).await {
                    // One login per connect, then the handshake runs again
                    // with the token the login stored.
                    Err(Login { challenge, expired }) => match &host.prompt {
                        Some(prompt) => {
                            auth::login(
                                url,
                                store.clone(),
                                Some(challenge),
                                &**prompt,
                                &host.app_name,
                            )
                            .await?;
                            Self::connect_http(name, url, store, timeout, handler)
                                .await
                                .map_err(|e| e.into_client(name, &host.login_command))
                        }
                        None => Err(ClientError::LoginRequired {
                            name: name.to_string(),
                            expired,
                            login_command: host.login_command.clone(),
                        }),
                    },
                    Err(e) => Err(e.into_client(name, &host.login_command)),
                    Ok(client) => Ok(client),
                }
            }
            (None, None) => Err(ClientError::NoTransport {
                name: name.to_string(),
            }),
        }
    }

    /// Opens a Streamable HTTP session with the stored token attached; a
    /// 401 on the handshake comes back as `Login` with the server's
    /// challenge, so the caller can decide whether anyone is there to log in.
    async fn connect_http(
        name: &str,
        url: &str,
        store: Arc<dyn CredentialStore>,
        timeout: Duration,
        handler: McpHandler,
    ) -> Result<Self, HttpError> {
        let (manager, expired) = auth::manager(url, store).await?;
        let client = AuthClient::new(reqwest::Client::new(), manager);
        let transport = StreamableHttpClientTransport::with_client(
            client,
            StreamableHttpClientTransportConfig::with_uri(url),
        );
        let asking = handler.asking.clone();
        let service = match handler.serve(transport).await {
            Ok(service) => service,
            Err(e) => {
                return Err(match challenge_of(&e) {
                    Some(challenge) => Login { challenge, expired },
                    None => HttpError::Handshake(Box::new(e)),
                });
            }
        };
        Ok(Self {
            name: name.to_string(),
            service: Arc::new(service),
            timeout,
            asking,
        })
    }

    /// Handshakes over any rmcp transport (tests use an in-process duplex).
    /// `ask` is the host's asker, `None` when nobody can answer.
    pub async fn from_transport<T, E, A>(
        name: &str,
        transport: T,
        timeout: Duration,
        ask: Option<Asker>,
    ) -> Result<Self, ClientError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let handler = McpHandler::new(name, ask, Arc::new(auth::open_browser));
        Self::serve(name, handler, transport, timeout).await
    }

    async fn serve<T, E, A>(
        name: &str,
        handler: McpHandler,
        transport: T,
        timeout: Duration,
    ) -> Result<Self, ClientError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let asking = handler.asking.clone();
        let service = handler
            .serve(transport)
            .await
            .map_err(|e| ClientError::Handshake(Box::new(e.into())))?;
        Ok(Self {
            name: name.to_string(),
            service: Arc::new(service),
            timeout,
            asking,
        })
    }

    /// Every tool the server lists, namespaced and (by default) deferred.
    pub async fn tools(&self, deferred: bool) -> Result<Vec<McpTool>, ClientError> {
        let listed = self
            .service
            .list_all_tools()
            .await
            .map_err(ClientError::List)?;
        Ok(listed
            .into_iter()
            .map(|tool| McpTool {
                client: self.clone(),
                tool,
                deferred,
            })
            .collect())
    }

    /// The server's name, as the host configured it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Ends the session; a stdio server's process is killed with it.
    pub async fn close(self) {
        if let Ok(service) = Arc::try_unwrap(self.service) {
            let _ = service.cancel().await;
        }
    }
}

/// A store that cannot be read (no keychain on this host, a locked secret
/// service) must not take the server down with it: the session keeps its
/// tokens in memory and `doctor` is where the keyring problem shows.
async fn usable(store: Arc<dyn CredentialStore>) -> Arc<dyn CredentialStore> {
    match store.load().await {
        Ok(_) => store,
        Err(_) => Arc::new(InMemoryCredentialStore::new()),
    }
}

use HttpError::Login;

/// Why an HTTP connect stopped; `Login` is the one the caller may recover
/// from. `expired` says credentials existed, so the notice can say "expired"
/// rather than "required".
enum HttpError {
    Login { challenge: String, expired: bool },
    Handshake(Box<ClientInitializeError>),
    Auth(AuthError),
}

impl From<AuthError> for HttpError {
    fn from(e: AuthError) -> Self {
        HttpError::Auth(e)
    }
}

impl HttpError {
    fn into_client(self, name: &str, login_command: &str) -> ClientError {
        match self {
            Login { expired, .. } => ClientError::LoginRequired {
                name: name.to_string(),
                expired,
                login_command: login_command.to_string(),
            },
            HttpError::Handshake(e) => ClientError::Handshake(Box::new((*e).into())),
            HttpError::Auth(e) => ClientError::Auth(e),
        }
    }
}

/// The `WWW-Authenticate` value when the handshake died on a 401/403. The
/// transport error is boxed as `dyn Error` inside rmcp's initialize error;
/// the concrete type is known because `connect_http` chose reqwest.
fn challenge_of(e: &ClientInitializeError) -> Option<String> {
    let ClientInitializeError::TransportError { error, .. } = e else {
        return None;
    };
    error
        .error
        .downcast_ref::<StreamableHttpError<reqwest::Error>>()
        .and_then(StreamableHttpError::auth_challenge)
        .map(str::to_string)
}

/// Connects every server; one that fails is a notice, not an error (fail open).
pub async fn connect_all(
    servers: &HashMap<String, ServerConfig>,
    timeout: Duration,
    deferred: bool,
    host: &Host,
) -> (Vec<McpClient>, Vec<McpTool>, Vec<String>) {
    let mut clients = Vec::new();
    let mut tools = Vec::new();
    let mut notices = Vec::new();
    let mut names: Vec<&String> = servers.keys().collect();
    names.sort();
    for name in names {
        // A login waits for a browser, so it gets its own budget on top of
        // the handshake timeout.
        let budget = match host.prompt {
            Some(_) => timeout + auth::LOGIN_TIMEOUT,
            None => timeout,
        };
        let connect = tokio::time::timeout(
            budget,
            McpClient::connect(name, &servers[name], timeout, host),
        );
        let listed = match connect.await {
            Ok(Ok(client)) => client.tools(deferred).await.map(|t| (client, t)),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                notices.push(skipped(
                    name,
                    &format!("no handshake within {}s", timeout.as_secs()),
                ));
                continue;
            }
        };
        match listed {
            Ok((client, list)) => {
                tools.extend(list);
                clients.push(client);
            }
            Err(e) => notices.push(skipped(name, &e.to_string())),
        }
    }
    (clients, tools, notices)
}

/// The notice [`connect_all`] leaves for a server it could not start;
/// [`skipped_server`] reads it back.
pub fn skipped(name: &str, reason: &str) -> String {
    format!("mcp server `{name}` skipped: {reason}")
}

/// The server and the reason in a [`skipped`] notice, so a surface can say
/// which server a session could not start without a second
/// channel beside the session's warnings. `None` for any other notice.
pub fn skipped_server(notice: &str) -> Option<(&str, &str)> {
    notice
        .strip_prefix("mcp server `")?
        .split_once("` skipped: ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const TOKEN: &str = "tok-1";

    /// The MCP side: answers `initialize` and `tools/list` for whatever id
    /// the client picked; notifications get the 202 the spec asks for.
    struct Rpc;

    impl Respond for Rpc {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let msg: Value = serde_json::from_slice(&req.body).unwrap_or_default();
            let result = match msg["method"].as_str() {
                Some("initialize") => json!({
                    "protocolVersion": msg["params"]["protocolVersion"],
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "t", "version": "0" },
                }),
                Some("tools/list") => json!({
                    "tools": [{ "name": "echo", "inputSchema": { "type": "object" } }]
                }),
                _ => return ResponseTemplate::new(202),
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({ "jsonrpc": "2.0", "id": msg["id"], "result": result }))
        }
    }

    /// The token endpoint: a code is good, a refresh is rejected for good.
    struct TokenEndpoint;

    impl Respond for TokenEndpoint {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            if String::from_utf8_lossy(&req.body).contains("grant_type=refresh_token") {
                return ResponseTemplate::new(400)
                    .set_body_json(json!({ "error": "invalid_grant" }));
            }
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": TOKEN, "token_type": "bearer",
                "expires_in": 3600, "refresh_token": "r-1",
            }))
        }
    }

    /// One wiremock plays both the authorization server and the MCP server:
    /// no bearer → 401 with the RFC 9728 pointer; metadata, registration
    /// and token endpoints; the right bearer → the JSON-RPC answers.
    async fn oauth_server() -> MockServer {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(Rpc)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/mcp"))
            .respond_with(
                ResponseTemplate::new(401).insert_header(
                    "www-authenticate",
                    format!(
                        "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\""
                    )
                    .as_str(),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/mcp"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-protected-resource"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": format!("{base}/mcp"),
                "authorization_servers": [base],
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "registration_endpoint": format!("{base}/register"),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "client_id": "cid", "redirect_uris": [],
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(TokenEndpoint)
            .mount(&server)
            .await;
        server
    }

    fn servers(base: &str) -> HashMap<String, ServerConfig> {
        HashMap::from([(
            "srv".to_string(),
            ServerConfig {
                url: Some(format!("{base}/mcp")),
                ..ServerConfig::default()
            },
        )])
    }

    /// The test is the browser: it reads `state` and `redirect_uri` off the
    /// authorization URL and hits the loopback callback with a code.
    fn browser() -> Prompt {
        Arc::new(|url: &str| {
            let parsed = reqwest::Url::parse(url).expect("authorization url");
            let param = |k: &str| {
                parsed
                    .query_pairs()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.into_owned())
                    .unwrap_or_else(|| panic!("{k} in {url}"))
            };
            assert_eq!(param("code_challenge_method"), "S256");
            let callback = format!(
                "{}?code=abc&state={}",
                param("redirect_uri"),
                param("state")
            );
            tokio::spawn(async move {
                let _ = reqwest::get(callback).await;
            });
        })
    }

    #[tokio::test]
    async fn oauth_401_then_token_then_200() {
        let server = oauth_server().await;
        let secrets = Arc::new(auth::Memory::default());
        let auth = Host {
            secrets: secrets.clone(),
            prompt: Some(browser()),
            ..Host::none()
        };
        let (clients, tools, notices) =
            connect_all(&servers(&server.uri()), Duration::from_secs(5), true, &auth).await;
        assert_eq!(notices, Vec::<String>::new());
        assert_eq!(clients.len(), 1);
        assert_eq!(
            tools
                .iter()
                .map(McpTool::qualified_name)
                .collect::<Vec<_>>(),
            ["mcp__srv__echo"]
        );
        let stored = secrets.store("srv").load().await.expect("load");
        let token =
            serde_json::to_value(stored.expect("credentials").token_response).expect("json");
        assert_eq!(token["access_token"], TOKEN);
    }

    #[tokio::test]
    async fn oauth_refresh_failure_is_a_warning() {
        let server = oauth_server().await;
        let secrets = Arc::new(auth::Memory::default());
        let dead = serde_json::from_value(json!({
            "access_token": "old", "token_type": "bearer",
            "expires_in": 1, "refresh_token": "dead",
        }))
        .expect("token response");
        secrets
            .store("srv")
            .save(rmcp::transport::auth::StoredCredentials::new(
                "cid".into(),
                Some(dead),
                vec![],
                Some(auth::now() - 100),
            ))
            .await
            .expect("save");
        let auth = Host {
            secrets,
            login_command: "app login".to_string(),
            ..Host::none()
        };
        // No prompt means no login allowance: the handshake timeout is the
        // whole budget, and a loaded machine stalled past 5 s turned this
        // into a "no handshake" notice (T22.8). The timeout is not the claim.
        let budget = Duration::from_secs(60);
        let (clients, tools, notices) =
            connect_all(&servers(&server.uri()), budget, true, &auth).await;
        assert!(clients.is_empty() && tools.is_empty());
        assert_eq!(
            notices,
            ["mcp server `srv` skipped: token expired, run `app login srv`"]
        );
    }

    #[test]
    fn a_skipped_notice_reads_back_its_server_and_reason() {
        let notice = skipped("srv", "spawn nope: not found");
        assert_eq!(
            skipped_server(&notice),
            Some(("srv", "spawn nope: not found"))
        );
        assert_eq!(skipped_server("mcp: .mcp.json skipped: bad json"), None);
    }
}
