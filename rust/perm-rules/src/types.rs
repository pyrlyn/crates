// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The values a decision is a function of. Plain data with no serde and no
//! host types: a host maps its own call and config types onto these.

/// How risky a call is, independent of what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    /// Cannot change anything the model does not already see.
    ReadOnly,
    /// Writes inside the workspace.
    Write,
    /// Runs a process.
    Exec,
    /// Can destroy data or reach beyond the immediate subject (`rm -rf`).
    Destructive,
}

/// The session-wide permission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Rules and risk decide as usual.
    Default,
    /// Only `Risk::ReadOnly` runs; everything else is denied without asking.
    Plan,
    /// `Write` runs without asking; `Exec` and `Destructive` still ask.
    Auto,
    /// Everything runs, except what a deny rule names.
    Bypass,
}

/// When the user is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalPolicy {
    /// Anything not covered by an `allow` rule asks.
    Untrusted,
    /// Risk-based asking.
    OnRequest,
    /// `Exec` runs confined without asking, while a sandbox exists.
    OnFailure,
    /// Every ask becomes a denial (headless).
    Never,
}

/// How much the host confines a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// No writes anywhere.
    ReadOnly,
    /// Writes confined to the workspace.
    WorkspaceWrite,
    /// No sandbox at all.
    DangerFullAccess,
}

/// What settled an `Allow` or a `Deny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    /// An `allow` or `deny` rule matched.
    Rule,
    /// A session grant matched.
    Session,
    /// The mode, the approval policy or the call's risk decided.
    Policy,
}

/// Why the surface must ask.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    /// An `ask` rule matched.
    RuleAsk {
        /// The rule as written.
        rule: String,
    },
    /// No rule matched; the call's risk requires asking.
    Risk {
        /// The call's risk.
        risk: Risk,
    },
    /// The approval policy forces asking regardless of risk.
    Policy {
        /// The policy in effect.
        policy: ApprovalPolicy,
    },
}

/// The simple commands a shell line splits into. The host owns the parser;
/// the engine only needs its verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Segments {
    /// Every simple command in source order, nested ones included.
    pub commands: Vec<String>,
    /// The split cannot vouch for the whole line (a substitution, `eval`,
    /// a parse error): no prefix rule or grant may allow it, while deny and
    /// ask rules still match `commands`.
    pub opaque: bool,
}

/// One call to judge. `tool` is an open name (`bash`, `edit`, `app`,
/// `site`, `mcp__srv__x`); nothing here enumerates the tools a host has.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The tool kind, matched case-insensitively after aliasing.
    pub tool: String,
    /// What rules match on: a path, a command line, a URL, an app id.
    pub subject: String,
    /// The call's risk classification.
    pub risk: Risk,
    /// Set for a shell line that splits into several commands.
    pub segments: Option<Segments>,
}

impl Call {
    /// A call whose subject is one unit.
    pub fn new(tool: impl Into<String>, subject: impl Into<String>, risk: Risk) -> Self {
        Self {
            tool: tool.into(),
            subject: subject.into(),
            risk,
            segments: None,
        }
    }
}
