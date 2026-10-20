# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The owner-only local RPC transport for a per-user agent, extracted for cofre
(T20) from aulo's listener: a Unix socket in a `0700` directory, a Windows named
pipe with a user-SID DACL, tonic `Connected` impls that report the peer, and a
login-service install (launchd, systemd --user, schtasks).

The directory mode, the socket mode, `first_pipe_instance`, the remote-client
rejection and the DACL are trust guards. A change that weakens one needs a
regression test next to it. Same-user malware is out of scope; say so, never
promise more.

## Commands

```bash
cargo test -p owner-listener
cargo clippy -p owner-listener --all-targets -- -D warnings
```

Tests that spawn a process or drop a listener and expect it gone take
`SPAWN_LOCK`: a forked child holds every open fd until it execs.
