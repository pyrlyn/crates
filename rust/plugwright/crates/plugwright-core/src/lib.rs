//! Shared vocabulary of plugwright: the [`Plugin`] trait every backend
//! implements, [`PluginError`], guest log forwarding into `tracing`, and
//! [`Worker`], a thread per plugin with two call lanes and deadlines.

pub mod error;
pub mod logging;
pub mod plugin;
pub mod worker;

pub use error::{PluginError, Result, WasmError, io_err};
pub use logging::{GUEST_TARGET, LogSink, tracing_sink};
pub use plugin::{
    Backend, Interrupt, Plugin, PluginExt, PluginInfo, Release, check_sdk, host_sdk_version, parse_version,
};
pub use worker::{CONTROL_DEPTH, EVENT_DEPTH, Lane, Worker};
