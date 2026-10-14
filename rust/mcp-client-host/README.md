# mcp-client-host

The client half of an MCP host over [`rmcp`](https://crates.io/crates/rmcp) 3.x.

- **Discovery.** `discover` merges the host's config, the project's `.mcp.json`
  and Claude Code's `~/.claude.json` (config wins), expands `${VAR}` and
  `${VAR:-default}`, and reports a broken file as a notice that names it.
- **Fail open.** `connect_all` connects every server, over stdio or Streamable
  HTTP, and returns the clients, their tools and one notice per server it had to
  skip. A failing server never stops the others.
- **OAuth.** A 401 on an HTTP server runs the authorization-code flow with PKCE
  through a loopback redirect; tokens live in a `Secrets` store the host injects
  (`auth::Memory` for tests, `auth::Keyring` behind the `keyring` feature).
- **Elicitation.** A server's `elicitation/create` form or URL request becomes
  one question at a time for a person, with a review step before anything is
  sent and consent before a URL is opened.
- **Tools.** Each server tool is an `McpTool` named `mcp__<server>__<tool>`,
  deferred by default, with its risk lowered only by `readOnlyHint`.

The licence is `GPL-3.0-or-later OR LicenseRef-Royalty-Free`
(see `LICENSE-ROYALTY-FREE.md`).
