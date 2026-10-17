// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The approval-policy × sandbox-mode table for `Exec` calls. Its own module
//! so the twelve cells are one function a matrix test reads directly, and a
//! host's sandbox-denial path consults the same table as the engine.

use crate::types::{ApprovalPolicy, SandboxMode};

/// How an `Exec` call that no rule or grant settled proceeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecPath {
    /// Run it confined without asking; a sandbox denial is the host's cue
    /// to ask and, on an allow, rerun it unconfined.
    Confined,
    /// Ask first.
    Ask,
    /// Refuse: the policy never asks and nothing confines the call.
    Deny,
}

/// `OnFailure` is the only policy that trusts the sandbox instead of the
/// user, and only while there is a sandbox: with `DangerFullAccess` it asks
/// like `OnRequest`. `Never` turns every ask into a denial.
pub fn exec_path(policy: ApprovalPolicy, sandbox: SandboxMode) -> ExecPath {
    match (policy, sandbox) {
        (ApprovalPolicy::OnFailure, SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite) => {
            ExecPath::Confined
        }
        (ApprovalPolicy::Never, _) => ExecPath::Deny,
        _ => ExecPath::Ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_failure_is_confined_only_while_a_sandbox_exists() {
        use ApprovalPolicy::{Never, OnFailure, Untrusted};
        use SandboxMode::{DangerFullAccess, ReadOnly, WorkspaceWrite};
        assert_eq!(exec_path(OnFailure, WorkspaceWrite), ExecPath::Confined);
        assert_eq!(exec_path(OnFailure, DangerFullAccess), ExecPath::Ask);
        assert_eq!(exec_path(Never, WorkspaceWrite), ExecPath::Deny);
        assert_eq!(exec_path(Untrusted, ReadOnly), ExecPath::Ask);
    }
}
