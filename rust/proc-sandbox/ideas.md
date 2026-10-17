# Ideas

- Serde derives (kebab-case) on `SandboxMode`, `LinuxBackend` and `SandboxPolicy`, behind a feature, so a host can read them straight from its config.
- Drop the `nix` dependency for `libc` directly: only its `libc` re-export is used.
- A macOS test that actually runs a command under Seatbelt and checks a write outside the roots fails.
