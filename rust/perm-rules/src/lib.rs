// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free

//! The single place a tool call is allowed, denied or escalated. Pure: rules
//! compile once from strings, then [`Engine::decide`] is a function of the
//! call, the mode, the policy and the session grants — no I/O, so a decision
//! table and a property test need no session around them. A tool never
//! checks its own permission.

pub mod policy;
pub mod rules;
mod types;

use std::path::Path;

use policy::{ExecPath, exec_path};
pub use rules::Grammar;
use rules::{Rule, tool_matches, word_prefix};
pub use types::{
    ApprovalPolicy, Call, DecidedBy, PermissionMode, Risk, SandboxMode, Segments, Why,
};

/// What the engine concluded for one call.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Run it.
    Allow {
        /// What allowed it.
        by: DecidedBy,
    },
    /// Refuse it; the model sees `reason`.
    Deny {
        /// Shown in the tool result.
        reason: String,
        /// What denied it.
        by: DecidedBy,
    },
    /// The surface must ask the user.
    Ask(Why),
}

/// The three rule lists as written, e.g. `Bash(git commit:*)`, `Edit(src/**)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSet {
    /// Rules that let a call run without asking.
    pub allow: Vec<String>,
    /// Rules that always ask.
    pub ask: Vec<String>,
    /// Rules that always refuse.
    pub deny: Vec<String>,
}

/// Which list a malformed rule came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleList {
    /// `allow`.
    Allow,
    /// `ask`.
    Ask,
    /// `deny`.
    Deny,
}

impl std::fmt::Display for RuleList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        })
    }
}

/// A malformed rule: a config error, never a silently skipped guard.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{list} rule {rule:?}: {message}")]
pub struct RuleError {
    /// The list holding the rule.
    pub list: RuleList,
    /// The rule as written.
    pub rule: String,
    /// What is wrong with it.
    pub message: String,
}

/// Compiled `allow`/`ask`/`deny` rules.
#[derive(Debug, Clone, Default)]
pub struct Engine {
    grammar: Grammar,
    deny: Vec<Rule>,
    allow: Vec<Rule>,
    ask: Vec<Rule>,
}

impl Engine {
    /// Compiles the three rule lists under `grammar`; the first malformed
    /// rule fails the whole set.
    pub fn compile(
        rules: &RuleSet,
        grammar: Grammar,
        home: Option<&Path>,
        cwd: &Path,
    ) -> Result<Self, RuleError> {
        let compile = |list: RuleList, raw: &[String]| {
            raw.iter()
                .map(|r| {
                    Rule::parse(r, &grammar, home, cwd).map_err(|message| RuleError {
                        list,
                        rule: r.clone(),
                        message,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Self {
            deny: compile(RuleList::Deny, &rules.deny)?,
            allow: compile(RuleList::Allow, &rules.allow)?,
            ask: compile(RuleList::Ask, &rules.ask)?,
            grammar,
        })
    }

    /// The decision order, first hit wins: deny rule; `Bypass` allows; `Plan`
    /// allows only read-only; allow rule; ask rule; session grant; `Untrusted`
    /// asks for anything not read-only; then the risk fallback. `Never` turns
    /// the final ask into a denial. `sandbox` only matters for `Exec` under
    /// `OnFailure`: that policy trusts the sandbox, so without one it asks.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    /// use perm_rules::*;
    ///
    /// let rules = RuleSet { deny: vec!["Read(~/.ssh/**)".into()], ..RuleSet::default() };
    /// let engine = Engine::compile(
    ///     &rules,
    ///     Grammar::default(),
    ///     Some(Path::new("/home/alice")),
    ///     Path::new("/repo"),
    /// )?;
    /// // A deny rule beats the ReadOnly default, whatever else matches.
    /// let call = Call::new("read", "/home/alice/.ssh/id_ed25519", Risk::ReadOnly);
    /// let outcome = engine.decide(
    ///     &call,
    ///     PermissionMode::Default,
    ///     ApprovalPolicy::OnRequest,
    ///     SandboxMode::WorkspaceWrite,
    ///     &[],
    /// );
    /// assert!(matches!(outcome, Outcome::Deny { .. }));
    /// # Ok::<(), RuleError>(())
    /// ```
    pub fn decide(
        &self,
        call: &Call,
        mode: PermissionMode,
        policy: ApprovalPolicy,
        sandbox: SandboxMode,
        grants: &[(String, String)],
    ) -> Outcome {
        let g = &self.grammar;
        // Deny and ask need one hit: the whole line or any of its commands.
        let first = |rules: &[Rule]| {
            rules
                .iter()
                .find(|r| {
                    r.matches(g, &call.tool, &call.subject)
                        || call
                            .segments
                            .as_ref()
                            .is_some_and(|s| s.commands.iter().any(|c| r.matches(g, &call.tool, c)))
                })
                .map(|r| r.raw.clone())
        };
        if let Some(rule) = first(&self.deny) {
            return Outcome::Deny {
                reason: format!("denied by rule {rule}"),
                by: DecidedBy::Rule,
            };
        }
        if mode == PermissionMode::Bypass {
            return Outcome::Allow {
                by: DecidedBy::Policy,
            };
        }
        if mode == PermissionMode::Plan {
            return if call.risk == Risk::ReadOnly {
                Outcome::Allow {
                    by: DecidedBy::Policy,
                }
            } else {
                Outcome::Deny {
                    reason: "plan mode: only read-only tools run; describe the change instead"
                        .into(),
                    by: DecidedBy::Policy,
                }
            };
        }
        if covered(
            call,
            |line| {
                self.allow
                    .iter()
                    .any(|r| r.matches_line(g, &call.tool, line))
            },
            |c| self.allow.iter().any(|r| r.matches(g, &call.tool, c)),
        ) {
            return Outcome::Allow {
                by: DecidedBy::Rule,
            };
        }
        let why = if let Some(rule) = first(&self.ask) {
            Some(Why::RuleAsk { rule })
        } else if self.granted(call, grants) {
            return Outcome::Allow {
                by: DecidedBy::Session,
            };
        } else if policy == ApprovalPolicy::Untrusted && call.risk != Risk::ReadOnly {
            Some(Why::Policy { policy })
        } else {
            by_risk(call.risk, mode, policy, sandbox)
        };
        match why {
            None => Outcome::Allow {
                by: DecidedBy::Policy,
            },
            Some(why) if policy == ApprovalPolicy::Never => Outcome::Deny {
                reason: format!("{} and the approval policy is `never`", why_text(&why)),
                by: DecidedBy::Policy,
            },
            Some(why) => Outcome::Ask(why),
        }
    }

    /// An `AllowForSession` grant. Grants are recorded per command by
    /// [`grants_for`]; a grant covers a command it prefixes at a word
    /// boundary, and an opaque line only when the user approved that exact
    /// line.
    fn granted(&self, call: &Call, grants: &[(String, String)]) -> bool {
        let mine = || {
            grants
                .iter()
                .filter(|(tool, _)| {
                    tool_matches(&self.grammar, &self.grammar.canonical(tool), &call.tool)
                })
                .map(|(_, subject)| subject.as_str())
        };
        // A call without segments is one subject (a path, a URL, an app id).
        // The same word boundary as a split command: `/repo/a.rs` does not
        // cover `/repo/a.rs.bak`, and `https://example.com` does not cover
        // `https://example.com.evil`. An empty grant is not a prefix of every
        // subject (`starts_with("")` is true for every string).
        covered(
            call,
            |line| mine().any(|g| g == line),
            |c| mine().any(|g| word_prefix(g, c)),
        )
    }
}

/// Whether allow-side matchers cover `call`. A call without segments is one
/// unit, matched by `each`. A split command line is covered by `line` on its
/// whole text (an exact or bare rule), or by `each` on every one of its
/// commands — never when the split is opaque.
fn covered(call: &Call, line: impl Fn(&str) -> bool, each: impl Fn(&str) -> bool) -> bool {
    match &call.segments {
        None => each(&call.subject),
        Some(s) => {
            line(&call.subject)
                || (!s.opaque && !s.commands.is_empty() && s.commands.iter().all(|c| each(c)))
        }
    }
}

/// The `(tool, subject)` grants an `AllowForSession` answer to `call`
/// records: one per command of a split line, so approving `git status &&
/// npm test` later covers `npm test` alone and never `npm test; rm -rf ~`.
/// An opaque line or a call without segments records its whole subject.
pub fn grants_for(call: &Call) -> Vec<(String, String)> {
    match &call.segments {
        Some(s) if !s.opaque && !s.commands.is_empty() => s
            .commands
            .iter()
            .map(|c| (call.tool.clone(), c.clone()))
            .collect(),
        _ => vec![(call.tool.clone(), call.subject.clone())],
    }
}

/// What the risk alone requires. An `Exec` call a host's classifier marked
/// safe arrives here as `ReadOnly`: risk is per call, not per tool.
fn by_risk(
    risk: Risk,
    mode: PermissionMode,
    policy: ApprovalPolicy,
    sandbox: SandboxMode,
) -> Option<Why> {
    match risk {
        Risk::ReadOnly => None,
        Risk::Write if mode == PermissionMode::Auto => None,
        Risk::Exec if exec_path(policy, sandbox) == ExecPath::Confined => None,
        _ => Some(Why::Risk { risk }),
    }
}

/// One line for a `Deny` reason or a notice.
pub fn why_text(why: &Why) -> String {
    match why {
        Why::RuleAsk { rule } => format!("rule {rule} requires approval"),
        Why::Risk { risk } => format!("{risk:?} calls require approval"),
        Why::Policy { policy } => format!("approval policy {policy:?} requires approval"),
    }
}

/// `Shift+Tab`: default → plan → auto → default; bypass is never cycled
/// into, only left. Here, beside the modes' meaning, so every surface
/// cycles in the same order.
pub fn next_mode(mode: PermissionMode) -> PermissionMode {
    match mode {
        PermissionMode::Default => PermissionMode::Plan,
        PermissionMode::Plan => PermissionMode::Auto,
        PermissionMode::Auto | PermissionMode::Bypass => PermissionMode::Default,
    }
}

/// The narrower of two permission modes: a preset may only tighten what the
/// configured mode allows, never widen it.
pub fn narrower(a: PermissionMode, b: PermissionMode) -> PermissionMode {
    if rank(a) <= rank(b) { a } else { b }
}

/// How much a mode lets through without asking: `Plan < Default < Auto <
/// Bypass`. The one definition of that order, so `narrower` cannot drift.
fn rank(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::Auto => 2,
        PermissionMode::Bypass => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_MODES: [PermissionMode; 4] = [
        PermissionMode::Plan,
        PermissionMode::Default,
        PermissionMode::Auto,
        PermissionMode::Bypass,
    ];

    #[test]
    fn narrower_never_returns_the_wider_mode() {
        for a in ALL_MODES {
            for b in ALL_MODES {
                let n = narrower(a, b);
                assert!(n == a || n == b, "{a:?} ∧ {b:?} gave a third mode {n:?}");
                assert_eq!(rank(n), rank(a).min(rank(b)), "{a:?} ∧ {b:?} gave {n:?}");
            }
        }
        assert_eq!(
            narrower(PermissionMode::Bypass, PermissionMode::Plan),
            PermissionMode::Plan
        );
        assert_eq!(
            narrower(PermissionMode::Auto, PermissionMode::Default),
            PermissionMode::Default
        );
    }

    #[test]
    fn narrower_is_commutative() {
        for a in ALL_MODES {
            for b in ALL_MODES {
                assert_eq!(narrower(a, b), narrower(b, a), "{a:?}, {b:?}");
            }
        }
    }

    #[test]
    fn shift_tab_cycles_default_plan_auto_and_leaves_bypass() {
        let from = [
            PermissionMode::Default,
            PermissionMode::Plan,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ];
        assert_eq!(
            from.map(next_mode),
            [
                PermissionMode::Plan,
                PermissionMode::Auto,
                PermissionMode::Default,
                PermissionMode::Default,
            ]
        );
    }
}
