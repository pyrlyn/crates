//! The one effort rule: which effort, if any, each wire sends for a request.
//! It depends on what a model declares ([`Capabilities`]), so no wire decides
//! for itself whether to send one. Each wire still turns the returned
//! [`Effort`] into its own generated enum; that is a type conversion the
//! compiler checks, not a policy.

use crate::model::Capabilities;
use crate::types::Effort;

/// The request shape a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// Anthropic Messages: `output_config.effort` and `thinking`.
    Anthropic,
    /// OpenAI Responses: `reasoning.effort`.
    Responses,
    /// OpenAI Chat Completions and the servers that speak it: top-level
    /// `reasoning_effort`.
    Chat,
    /// TypeSafe Jev: a decision model with no effort field.
    Jev,
}

/// What a wire sends for a request's effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireEffort {
    /// The level to send, as the router already clamped it.
    pub effort: Effort,
    /// Anthropic only: send `thinking: {"type": "adaptive"}` when the tier
    /// asks for thinking. Always `false` on the other wires.
    pub adaptive_thinking: bool,
}

/// The effort `api` sends for `effort` on a model with `caps`, or `None`
/// when that wire sends no effort field at all.
///
/// - Anthropic always sends `output_config.effort`; adaptive thinking
///   follows `caps.adaptive_thinking`.
/// - Responses always sends `reasoning.effort`.
/// - Chat sends `reasoning_effort` only when the row declares
///   `reasoning_effort_param`: OpenAI's Chat API documents the field, but
///   LM Studio's compatible endpoint does not list it (research.md §4.3.3).
/// - Jev has no effort field.
pub fn effort_for(api: Api, effort: Effort, caps: &Capabilities) -> Option<WireEffort> {
    let plain = WireEffort {
        effort,
        adaptive_thinking: false,
    };
    match api {
        Api::Anthropic => Some(WireEffort {
            adaptive_thinking: caps.adaptive_thinking == Some(true),
            ..plain
        }),
        Api::Responses => Some(plain),
        Api::Chat => (caps.reasoning_effort_param == Some(true)).then_some(plain),
        Api::Jev => None,
    }
}

/// Model-id prefixes that take Anthropic's `thinking: {"type": "adaptive"}`
/// field. Older models want `{"type": "enabled", "budget_tokens": N}`,
/// which is a 400 on these — no wire here sends `budget_tokens`, so an
/// unlisted model simply gets no `thinking` field.
///
/// It stays a plain prefix rule rather than a `Capabilities`
/// field: no data source emits an adaptive-thinking signal today
/// (`Capabilities::adaptive_thinking` is `None` on every row), and a
/// row-based lookup would silently stop matching a model id that names no
/// catalog row at all (a preview or custom variant the prefix table has
/// always matched by name).
const ADAPTIVE_THINKING_PREFIXES: &[&str] = &[
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-haiku-5",
    "claude-fable-5",
    "claude-mythos-5",
    "claude-opus-4-6",
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-sonnet-4-6",
];

/// Whether `model_id` takes Anthropic's adaptive `thinking` field (see
/// [`ADAPTIVE_THINKING_PREFIXES`]).
pub fn supports_adaptive_thinking(model_id: &str) -> bool {
    ADAPTIVE_THINKING_PREFIXES
        .iter()
        .any(|p| model_id.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Effort; 4] = [Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh];

    fn caps(adaptive: Option<bool>, param: Option<bool>) -> Capabilities {
        Capabilities {
            adaptive_thinking: adaptive,
            reasoning_effort_param: param,
            ..Capabilities::default()
        }
    }

    /// `(api, caps.adaptive_thinking, caps.reasoning_effort_param, sent)`;
    /// `sent` is `None` when the wire sends no effort, `Some(t)` when it
    /// sends the level with adaptive thinking `t`.
    type Row = (Api, Option<bool>, Option<bool>, Option<bool>);

    const TABLE: &[Row] = &[
        (Api::Anthropic, None, None, Some(false)),
        (Api::Anthropic, Some(false), None, Some(false)),
        (Api::Anthropic, Some(true), None, Some(true)),
        (Api::Anthropic, None, Some(true), Some(false)),
        (Api::Responses, None, None, Some(false)),
        (Api::Responses, Some(true), None, Some(false)),
        (Api::Responses, None, Some(false), Some(false)),
        (Api::Chat, None, None, None),
        (Api::Chat, None, Some(false), None),
        (Api::Chat, Some(true), None, None),
        (Api::Chat, None, Some(true), Some(false)),
        (Api::Chat, Some(true), Some(true), Some(false)),
        (Api::Jev, None, None, None),
        (Api::Jev, Some(true), Some(true), None),
    ];

    #[test]
    fn effort_for_matches_the_api_effort_caps_table() {
        for effort in ALL {
            for &(api, adaptive, param, want) in TABLE {
                let want = want.map(|adaptive_thinking| WireEffort {
                    effort,
                    adaptive_thinking,
                });
                assert_eq!(
                    effort_for(api, effort, &caps(adaptive, param)),
                    want,
                    "{api:?} {effort:?} adaptive={adaptive:?} param={param:?}"
                );
            }
        }
    }

    #[test]
    fn supports_adaptive_thinking_matches_listed_prefixes_only() {
        assert!(supports_adaptive_thinking("claude-sonnet-5"));
        // Prefix match, not exact match: a dated/preview suffix still hits.
        assert!(supports_adaptive_thinking("claude-sonnet-5-20260115"));
        // Not listed: an older/unlisted family gets no `thinking` field.
        assert!(!supports_adaptive_thinking("claude-haiku-4-5"));
        assert!(!supports_adaptive_thinking("gpt-5.1"));
    }
}
