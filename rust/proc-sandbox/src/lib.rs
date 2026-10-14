// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! Confinement for a child process and for the paths handed to it.
//!
//! [`sandbox`] turns a [`SandboxPolicy`] into what confines a command on this
//! host: Seatbelt on macOS, bubblewrap or Landlock plus seccomp on Linux.
//! [`path::confine`] is the matching guard for a path string that comes from
//! an untrusted caller. Both are trust guards, so they live apart from any
//! tool set and share one small policy type, [`policy`].

// why: tests assert with `expect`/`unwrap`; the deny applies to shipped code.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod path;
pub mod policy;
pub mod sandbox;

pub use policy::{LinuxBackend, SandboxMode, SandboxPolicy};
