# owner-listener

Owner-only local RPC transport for a per-user agent.

* On Unix, [`unix::bind`] binds a socket in a `0700` directory, sets the socket
  to `0600`, and reports the peer uid and pid on each connection through
  tonic's `Connected` trait.
* On Windows, [`pipe::bind`] creates a named pipe with `first_pipe_instance`,
  rejects remote clients, and sets a DACL that allows only the current user's
  SID. Each connection carries the client pid. [`pipe::connect`] reports the
  server pid so the client can check it is talking to the process it started.
* [`service`] installs that process as a login service: a LaunchAgent, a
  systemd user unit, or a Task Scheduler logon task. The label, unit name and
  home variable are arguments, so the crate is not tied to one product.

Same-user malware is out of scope. Any process of the owning user can connect.
The directory mode and the DACL are what stop other users.

This crate does not check an Authenticode signature. The caller compares the
server pid with the agent it trusts and, if it wants a signature check, applies
its own signer policy.

aulo still ships its own listener. Moving aulo onto this crate is that
repository's decision.

The crate is not published to crates.io. Depend on it with a git pin:

```toml
owner-listener = { git = "https://github.com/pyrlyn/crates", rev = "<commit>" }
```
