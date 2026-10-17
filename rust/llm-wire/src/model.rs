//! What a host declares about a model, and what that declares the model can
//! do. The wires and the catalog read the same two types, so a configured
//! model entry is held once however many crates look at it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::Effort;

/// One configured model: what a host's `[providers.*].models` entry (or a
/// built-in `models.toml` row) says about a model id. The host fills these
/// from its own config, so this crate depends on no config type of the
/// host's. An empty `efforts` means "any".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct ProviderModel {
    /// The model id sent on the wire (for gateways the full `vendor/model`
    /// id, e.g. `"anthropic/claude-sonnet-5"`).
    pub id: String,
    /// What a person calls the model (`"Claude Sonnet 5"`). Unset means a
    /// reader shows the id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Context window in tokens; `0` means not declared.
    pub context_window: u32,
    /// Efforts this model supports; empty means "any".
    pub efforts: Vec<Effort>,
    /// Whether this model takes the Chat Completions `reasoning_effort`
    /// field. Unset means "not declared", and a chat wire then sends no
    /// effort at all: OpenAI documents the field, LM Studio's compatible
    /// endpoint does not list it, so it is opt-in per model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<bool>,
    /// Whether this model takes image input. `false` makes the Chat wire
    /// refuse a request that carries an image; unset still sends it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
}

/// What a model is known to support. Every field is `None` until a data
/// source supplies it, so a model with nothing declared is `Default`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Capabilities {
    /// Accepts tool definitions on a request.
    pub tools: Option<bool>,
    /// Sends extended/adaptive thinking.
    pub adaptive_thinking: Option<bool>,
    /// Accepts a reasoning-effort wire parameter. Declared per model by
    /// [`ProviderModel::reasoning_effort`]; read by
    /// [`effort_for`](crate::effort_for) for the Chat wire.
    pub reasoning_effort_param: Option<bool>,
    /// Accepts image input. Declared per model by [`ProviderModel::images`].
    pub images: Option<bool>,
}

impl Capabilities {
    /// What a configured entry declares. A wire that holds only its
    /// section's entries (Chat) and the catalog merge both read it here,
    /// so the two cannot disagree.
    pub fn declared_by(model: &ProviderModel) -> Self {
        Self {
            reasoning_effort_param: model.reasoning_effort,
            images: model.images,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_by_reads_the_model_entry() {
        let model = ProviderModel {
            id: "m".into(),
            reasoning_effort: Some(true),
            images: Some(false),
            ..Default::default()
        };
        assert_eq!(
            Capabilities::declared_by(&model),
            Capabilities {
                reasoning_effort_param: Some(true),
                images: Some(false),
                ..Capabilities::default()
            }
        );
    }
}
