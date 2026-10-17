# proc-sandbox

Confine a child process, and the paths handed to it.

- `sandbox::command(policy, roots, writable_roots, shell, command)` builds a
  `std::process::Command` for `<shell> -c <command>` under the policy.
- `sandbox::argv(policy, roots, writable_roots, program)` returns the wrapped
  argv for a program that only has an argv (an interactive shell in a PTY).
- `sandbox::backend(linux)` says what confines commands on this host.
- `path::confine(roots, cwd, input)` resolves a path string from an untrusted
  caller and refuses anything that escapes the roots, including through
  symlinks, `..`, NUL bytes and Windows-only syntax.

| Host | Backend | How |
| --- | --- | --- |
| macOS | Seatbelt | `/usr/bin/sandbox-exec` with a generated profile |
| Linux | bubblewrap | user and pid namespaces, read-only `/`, writable binds, no network namespace unless allowed |
| Linux | Landlock + seccomp | filesystem rules and a network filter applied in the child between fork and exec |
| other | none | the command runs bare; the caller decides whether that is acceptable |

`SandboxMode::DangerFullAccess` always runs bare. The Landlock backend cannot
express a read-only subpath inside a writable root and cannot wrap an argv;
`argv` refuses it with `ErrorKind::Unsupported`.

```rust
use std::path::{Path, PathBuf};
use proc_sandbox::{sandbox, LinuxBackend, SandboxMode, SandboxPolicy};

let policy = SandboxPolicy {
    mode: SandboxMode::WorkspaceWrite,
    network: false,
    writable: vec![],
    readonly_in_workspace: vec![PathBuf::from(".git")],
    linux_backend: LinuxBackend::Auto,
};
let ws = vec![PathBuf::from("/work/project")];
let cmd = sandbox::command(&policy, &ws, &ws, Path::new("/bin/sh"), "ls")?;
# Ok::<(), std::io::Error>(())
```

The licence is `GPL-3.0-or-later OR LicenseRef-Royalty-Free`
(see `LICENSE-ROYALTY-FREE.md`).
