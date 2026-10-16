// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The sandbox's input types. Plain data with no serde, so the crate has no
//! opinion on how a host reads them from its config.

use std::path::PathBuf;

/// How much of the filesystem a command may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// No writes anywhere but scratch space.
    ReadOnly,
    /// Writes confined to the workspace roots.
    WorkspaceWrite,
    /// No sandbox at all; requires explicit opt-in.
    DangerFullAccess,
}

/// Which Linux confinement to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LinuxBackend {
    /// `bwrap` when it can create namespaces here, else Landlock, else none.
    #[default]
    Auto,
    /// bubblewrap only.
    Bwrap,
    /// Landlock + seccomp only.
    Landlock,
    /// No confinement on Linux.
    None,
}

/// What one command runs under.
#[derive(Debug, Clone, PartialEq)]
pub struct SandboxPolicy {
    /// The sandbox mode in effect.
    pub mode: SandboxMode,
    /// Whether network access is allowed.
    pub network: bool,
    /// Extra writable roots beyond the workspace.
    pub writable: Vec<PathBuf>,
    /// Paths inside a writable root that stay read-only (`.git`).
    pub readonly_in_workspace: Vec<PathBuf>,
    /// Which Linux backend confines the command; ignored elsewhere.
    pub linux_backend: LinuxBackend,
}
