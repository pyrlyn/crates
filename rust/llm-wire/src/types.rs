//! The provider-neutral request/response shapes: `Request` in, a stream of
//! `ProviderEvent`s out, plus every type reachable from them. Only shapes and
//! their serde/schemars derives live here, so any crate can depend on the
//! provider contract without depending on a provider.
//!
//! Serde convention: struct-shaped enums use `#[serde(tag = "type",
//! rename_all = "snake_case")]` so a rollout line greps as
//! `"type":"tool_use_start"`; small field-less enums use bare
//! `#[serde(rename_all = "snake_case")]`, serializing as a plain string.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::ProviderError;
use crate::ids::{ArchiveId, CallId};

/// How risky a tool call is, independent of what it does (plan.md §1.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Cannot change anything cox does not already show the model.
    ReadOnly,
    /// Writes inside the workspace.
    Write,
    /// Runs a process.
    Exec,
    /// Can destroy data or affect more than the immediate subject (`rm -rf`, `apply_patch` deleting > 5 files).
    Destructive,
}

/// Whether a tool may run alongside other calls in the same batch (plan.md §1.3 step 3.d.iv).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
    /// May run in parallel with other `Parallel` calls, up to `core.parallel_tools`.
    Parallel,
    /// Must run alone; other calls in the batch wait.
    Exclusive,
}

/// A routing tier (plan.md §1.4/D5): a job maps to a tier, a tier maps to a model.
///
/// Ordered cheapest first (declaration order), the one tier ordering every
/// "never up" rule compares with: a plugin's model-call grant clamp
/// (PL§7d) and the `route` decision point (PL§4, T33.20).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Haiku-class or local; mechanical work, never chosen for the main coding turn.
    Cheap,
    /// Sonnet by default, Opus when picked; the main coding turn.
    Code,
    /// Fable 5.1 only, only via `/think`/`--deep`, always confirmed.
    Think,
}

/// What a request is *for* (plan.md §1.4); every job is pinned to one tier in config.
///
/// Every variant but `Plugin` is a bare tag (`"main"`, `"compact"`, …), the
/// convention this file's header describes for field-less enums. `Plugin`
/// breaks that shape — it carries the calling plugin's id — so `Job` gets
/// hand-written `Serialize`/`Deserialize`/`JsonSchema` instead of deriving
/// them: the wire and ledger form is still a single string, `"plugin:<id>"`
/// (PL§7d, T33.15), which `to_tag`/`from_tag` (`cox-store`) and `tag`
/// (`cox`'s `stats.rs`) already assume for every `Job` value. Losing `Copy`
/// (a `String` payload cannot be `Copy`) is why call sites that used to
/// read `self.job`/`row.job` as a value now clone it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// The main coding turn.
    Main,
    /// A `/think`/`--deep` plan.
    Plan,
    /// Compaction summary.
    Compact,
    /// Session title generation.
    Title,
    /// Tool-result or transcript summarisation.
    Summarize,
    /// Commit message generation.
    Commit,
    /// Memory extraction.
    Memory,
    /// An `explore` subagent.
    Explore,
    /// A background shell/HTTP subagent.
    Shell,
    /// A custom subagent definition (`.cox/agents`/`.claude/agents`,
    /// T34.1): its own `tier`/`model` decides the tier, not this job.
    Agent,
    /// A hook-driven LLM call.
    Hook,
    /// A plugin's own `cox_model_call` (PL§7d, T33.15): the router runs it
    /// at or below the plugin's granted tier, never `think`, and the
    /// ledger row's job tag is `plugin:<id>`. No `[jobs]` entry names it —
    /// its tier comes from the call itself, already grant-clamped.
    Plugin(String),
}

/// The bare tag every non-`Plugin` variant serializes as — kept in one
/// place so the `Serialize`/`Deserialize`/`JsonSchema` impls below and the
/// schema literals agree.
const JOB_TAGS: [(&str, Job); 11] = {
    // A `const` array can't hold a `String`-carrying variant, so this only
    // ever binds the fieldless ones; `Job::Plugin` is handled separately
    // everywhere this table is used.
    [
        ("main", Job::Main),
        ("plan", Job::Plan),
        ("compact", Job::Compact),
        ("title", Job::Title),
        ("summarize", Job::Summarize),
        ("commit", Job::Commit),
        ("memory", Job::Memory),
        ("explore", Job::Explore),
        ("shell", Job::Shell),
        ("agent", Job::Agent),
        ("hook", Job::Hook),
    ]
};

impl Job {
    /// The bare tag this job serializes as: one of the fixed strings above,
    /// or `plugin:<id>` for `Job::Plugin`.
    fn tag(&self) -> std::borrow::Cow<'static, str> {
        match self {
            Job::Plugin(id) => std::borrow::Cow::Owned(format!("plugin:{id}")),
            other => JOB_TAGS
                .iter()
                .find(|(_, job)| job == other)
                .map(|(tag, _)| std::borrow::Cow::Borrowed(*tag))
                .unwrap_or(std::borrow::Cow::Borrowed("")),
        }
    }
}

impl Serialize for Job {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.tag())
    }
}

impl<'de> Deserialize<'de> for Job {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if let Some((_, job)) = JOB_TAGS.iter().find(|(tag, _)| *tag == s) {
            return Ok(job.clone());
        }
        if let Some(id) = s.strip_prefix("plugin:")
            && !id.is_empty()
        {
            return Ok(Job::Plugin(id.to_string()));
        }
        Err(serde::de::Error::unknown_variant(
            &s,
            &[
                "main",
                "plan",
                "compact",
                "title",
                "summarize",
                "commit",
                "memory",
                "explore",
                "shell",
                "agent",
                "hook",
                "plugin:<id>",
            ],
        ))
    }
}

impl JsonSchema for Job {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("Job")
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "What a request is *for* (plan.md §1.4); every job is pinned to one tier in config.",
            "oneOf": [
                { "description": "The main coding turn.", "type": "string", "const": "main" },
                { "description": "A `/think`/`--deep` plan.", "type": "string", "const": "plan" },
                { "description": "Compaction summary.", "type": "string", "const": "compact" },
                { "description": "Session title generation.", "type": "string", "const": "title" },
                { "description": "Tool-result or transcript summarisation.", "type": "string", "const": "summarize" },
                { "description": "Commit message generation.", "type": "string", "const": "commit" },
                { "description": "Memory extraction.", "type": "string", "const": "memory" },
                { "description": "An `explore` subagent.", "type": "string", "const": "explore" },
                { "description": "A background shell/HTTP subagent.", "type": "string", "const": "shell" },
                {
                    "description": "A custom subagent definition (`.cox/agents`/`.claude/agents`,\nT34.1): its own `tier`/`model` decides the tier, not this job.",
                    "type": "string",
                    "const": "agent"
                },
                { "description": "A hook-driven LLM call.", "type": "string", "const": "hook" },
                {
                    "description": "A plugin's own `cox_model_call` (PL§7d, T33.15): the router runs it\nat or below the plugin's granted tier, never `think`, and the\nledger row's job tag is `plugin:<id>`. No `[jobs]` entry names it —\nits tier comes from the call itself, already grant-clamped.",
                    "type": "string",
                    "pattern": "^plugin:.+$"
                }
            ]
        })
    }
}

/// Reasoning effort passed to the provider.
///
/// Ordered `Low < Medium < High < Xhigh` so the router can clamp a tier's
/// effort to the greatest level a model supports (`docs/design/providers.md`).
/// The four levels are models.dev's own, so a catalog row maps without loss
/// (T30.26).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// Cheapest, fastest; used for `cheap`-tier jobs.
    Low,
    /// Between `Low` and `High`: models.dev's and the wires' `medium`.
    Medium,
    /// Default for `code`/`think` tiers.
    High,
    /// User-selected for a flagged large refactor.
    Xhigh,
}

impl Effort {
    /// The lowercase name the config, the wire and `/effort` share.
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        }
    }

    /// The inverse of `name`; anything else is `None`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            _ => None,
        }
    }
}

/// Extended/adaptive thinking mode for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Thinking {
    /// No thinking block requested.
    Off,
    /// Provider decides whether and how much to think.
    Adaptive,
}

/// Why a provider call or a turn stopped.
///
/// Reused for both `ProviderEvent::Stop` (one provider call) and
/// `Event::TurnDone` (the whole turn); a provider only ever emits
/// `EndTurn`/`Refusal`/`Error`, the others are added by `cox-core` once it
/// has aggregated multiple calls in a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished normally.
    EndTurn,
    /// `core.max_turns` provider calls were used up without finishing.
    MaxTurns,
    /// `Submission::Interrupt` cancelled the turn.
    Interrupted,
    /// A budget cap stopped the turn before another call was made.
    Budget,
    /// The model refused to continue.
    Refusal {
        /// The provider's refusal text, if any.
        detail: String,
    },
    /// The turn ended in an unrecoverable error.
    Error,
}

/// A pointer to a full tool output stored in the archive (plan.md §1.7/D6a).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveRef {
    /// The archive row's id (`cox expand <id>`).
    pub id: ArchiveId,
    /// Size of the archived payload, in bytes.
    pub bytes: u64,
}

/// Per-request token/cost accounting (plan.md §1.2/§1.9); one row per provider call.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    /// Input tokens billed at the full rate.
    pub input_tokens: u32,
    /// Output tokens generated.
    pub output_tokens: u32,
    /// Input tokens served from cache (billed at the cache-read rate).
    pub cache_read_tokens: u32,
    /// Input tokens written to cache (billed at the cache-write rate).
    pub cache_write_tokens: u32,
    /// True when the provider reported no usage and cox estimated it.
    pub estimated: bool,
    /// Computed cost of this call, in USD.
    pub cost_usd: f64,
    /// Wall-clock latency of this call.
    pub latency_ms: u64,
}

impl Usage {
    /// Tokens the model actually saw for this call: input + cache read + cache write
    /// (plan.md §1.9: "context_tokens ... writes ... (input + cache read + cache write)").
    /// Excludes `output_tokens`, which the model produced rather than read.
    pub fn context_tokens(&self) -> u32 {
        self.input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens)
    }
}

/// A newtype around a provider's model identifier (e.g. `"claude-sonnet-5"`).
/// Deliberately a bare string, not an enum: models are configured, not
/// compiled in (`config/default.toml` [tiers.*].model).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ModelId(pub String);

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which provider backend is in play; matches the `[providers.*]` config sections (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    /// Anthropic Messages API.
    Anthropic,
    /// OpenAI Responses or Chat Completions API.
    OpenAi,
    /// A local OpenAI-compatible server (Ollama, vLLM, LM Studio, …).
    Local,
    /// TypeSafe System One API (Jev decision model, T21.1).
    Jev,
    /// An external CLI agent from a plugin (EA§6, T35.5): billed on the
    /// user's own plan, so its ledger rows are `$0` and never priced here.
    External,
}

/// One block of the system prompt, with its own cache eligibility (plan.md §1.9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemBlock {
    /// The block's text.
    pub text: String,
    /// Whether this block may sit before a cache breakpoint.
    pub cache: bool,
}

/// Who sent a `Message` in a `Request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The end user (also carries tool results, by provider convention).
    User,
    /// The model.
    Assistant,
}

/// One piece of a `Message`'s content (plan.md §1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    /// Plain text.
    Text {
        /// The text.
        text: String,
    },
    /// An extended-thinking block, replayed back to providers that require it verbatim.
    Thinking {
        /// The thinking text.
        text: String,
        /// The provider's signature for this block, if required.
        signature: Option<String>,
    },
    /// The model's request to use a tool.
    ToolUse {
        /// The call's id.
        id: CallId,
        /// The tool name.
        name: String,
        /// The (possibly still-accumulating) input.
        input: Value,
    },
    /// A tool's result, sent back to the model.
    ToolResult {
        /// Which call this answers.
        call_id: CallId,
        /// The result content (already truncated/sanitised for the model).
        content: String,
        /// Whether the tool call failed.
        is_error: bool,
    },
    /// An inline image.
    Image {
        /// MIME type.
        media_type: String,
        /// Base64-encoded bytes.
        data_b64: String,
    },
    /// A reference to archived content instead of the content itself (microcompaction).
    Pointer {
        /// Where the full content lives.
        archive: ArchiveRef,
        /// A short description shown in its place.
        summary: String,
    },
}

/// One message in a `Request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    /// Who sent it.
    pub role: Role,
    /// Its content blocks.
    pub content: Vec<Content>,
}

/// A provider-neutral request; providers translate this to their own wire
/// format, and nothing above `cox-provider` knows what that format is
/// (plan.md §1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    /// The routing tier this request was assembled for.
    pub tier: Tier,
    /// The job this request serves.
    pub job: Job,
    /// The specific model to call.
    pub model: ModelId,
    /// System prompt blocks, in cache-stable order (plan.md §1.9).
    pub system: Vec<SystemBlock>,
    /// Available tool specs, already filtered (deferred tools absent unless discovered).
    pub tools: Vec<ToolSpec>,
    /// The conversation so far.
    pub messages: Vec<Message>,
    /// Requested reasoning effort.
    pub effort: Effort,
    /// Max output tokens.
    pub max_tokens: u32,
    /// Extended-thinking mode.
    pub thinking: Thinking,
    /// Indices into `system` + `messages` (in that concatenated order) marking cache breakpoints; at most 3.
    pub cache_breakpoints: Vec<usize>,
    /// Sequences that stop generation.
    pub stop_sequences: Vec<String>,
}

/// One event from a provider's streamed response (plan.md §1.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderEvent {
    /// The stream started; names the model that answered (may differ from the requested alias).
    MessageStart {
        /// The model that is responding.
        model: ModelId,
    },
    /// The next chunk of assistant text.
    TextDelta {
        /// The text chunk.
        text: String,
    },
    /// The next chunk of thinking text.
    ThinkingDelta {
        /// The text chunk.
        text: String,
    },
    /// A tool-use block started.
    ToolUseStart {
        /// The call's id.
        id: CallId,
        /// The tool name.
        name: String,
    },
    /// The current tool-use block's thought signature (Gemini over the Chat
    /// wire). Opaque: cox never reads it, and it is replayed only to the wire
    /// that produced it. Follows its call's `ToolUseStart`, before `ToolUseEnd`.
    ToolUseSignature {
        /// The signature, byte-for-byte as received.
        signature: String,
    },
    /// The next chunk of a tool-use block's JSON input.
    ToolUseInputDelta {
        /// The raw JSON chunk (accumulate and parse once `ToolUseEnd` arrives).
        text: String,
    },
    /// The current tool-use block finished.
    ToolUseEnd,
    /// The stream stopped.
    Stop {
        /// Why it stopped.
        stop: StopReason,
    },
    /// Final usage for this call.
    Usage {
        /// The recorded usage.
        usage: Usage,
    },
    /// The provider is retrying after a transient failure.
    Retrying {
        /// Which retry attempt this is (1-based).
        attempt: u32,
        /// How long cox waited before this attempt.
        after_ms: u64,
    },
    /// The call failed.
    Error {
        /// The failure.
        error: ProviderError,
    },
}

/// What a provider implementation can do; used to skip unsupported request shapes
/// instead of sending them and getting `ProviderError::Unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Caps {
    /// Supports prompt caching (`cache_control`/automatic prefix caching).
    pub cache: bool,
    /// Supports extended/adaptive thinking.
    pub thinking: bool,
    /// Supports first-party server tools (web search/fetch passthrough).
    pub server_tools: bool,
    /// Supports a dedicated token-counting endpoint.
    pub count_tokens: bool,
    /// The model's max context window, in tokens.
    pub max_context: u32,
}

/// A tool's advertised shape (plan.md §1.2/§1.11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolSpec {
    /// The tool's registered name.
    pub name: String,
    /// Shown to the model.
    pub description: String,
    /// JSON Schema for the tool's input.
    pub input_schema: Value,
    /// True for tools found only through `tool_search`, absent from `system[0]` until discovered.
    pub deferred: bool,
    /// Default risk classification for calls to this tool.
    pub risk: Risk,
    /// Whether calls to this tool may run in parallel with others.
    pub concurrency: Concurrency,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    fn sample_usage() -> Usage {
        Usage {
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 30,
            cache_write_tokens: 5,
            estimated: false,
            cost_usd: 0.01,
            latency_ms: 250,
        }
    }

    #[test]
    fn effort_medium_sits_between_low_and_high_and_round_trips_by_name() {
        let all = [Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh];
        assert!(all.windows(2).all(|w| w[0] < w[1]));
        for e in all {
            assert_eq!(Effort::parse(e.name()), Some(e));
            assert_eq!(serde_json::to_value(e).ok(), Some(e.name().into()));
        }
        assert_eq!(Effort::Medium.name(), "medium");
    }

    #[test]
    fn usage_sums_cache_fields() {
        let usage = sample_usage();
        assert_eq!(usage.context_tokens(), 100 + 30 + 5);
    }

    /// Every fieldless `Job` still serializes as its bare tag (unchanged by
    /// the hand-written impl), and `Plugin` serializes/round-trips as
    /// `plugin:<id>`: the literal ledger tag cox's store and stats rely on.
    #[test]
    fn job_tags_are_plain_strings_and_plugin_round_trips_by_id() {
        assert_eq!(serde_json::to_value(Job::Main).unwrap(), "main");
        assert_eq!(serde_json::to_value(Job::Hook).unwrap(), "hook");
        assert_eq!(
            serde_json::to_value(Job::Plugin("git-glance".into())).unwrap(),
            "plugin:git-glance"
        );
        let round: Job = serde_json::from_value(serde_json::json!("plugin:git-glance")).unwrap();
        assert_eq!(round, Job::Plugin("git-glance".into()));
        let round: Job = serde_json::from_value(serde_json::json!("main")).unwrap();
        assert_eq!(round, Job::Main);
        assert!(serde_json::from_value::<Job>(serde_json::json!("plugin:")).is_err());
        assert!(serde_json::from_value::<Job>(serde_json::json!("bogus")).is_err());
    }

    #[rstest]
    #[case::tool_use_start(ProviderEvent::ToolUseStart { id: CallId::new(), name: "read".into() })]
    #[case::tool_use_signature(ProviderEvent::ToolUseSignature { signature: "sig-opaque".into() })]
    #[case::tool_use_end(ProviderEvent::ToolUseEnd)]
    fn provider_event_json_roundtrip(#[case] event: ProviderEvent) {
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ProviderEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, back);
    }

    #[test]
    fn tool_spec_schema_generates() {
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct ReadInput {
            path: String,
            lines: Option<String>,
        }

        let schema = schemars::schema_for!(ReadInput);
        let schema_value = serde_json::to_value(&schema).expect("schema serializes");
        let spec = ToolSpec {
            name: "read".into(),
            description: "Read a file".into(),
            input_schema: schema_value.clone(),
            deferred: false,
            risk: Risk::ReadOnly,
            concurrency: Concurrency::Parallel,
        };
        assert_eq!(spec.input_schema, schema_value);
        assert!(schema_value.get("properties").is_some());
    }
}
