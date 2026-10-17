//! Shared vocabulary of plugwright: the [`Plugin`] trait every backend
//! implements, [`PluginError`], and guest log forwarding into `tracing`.

pub mod error;
pub mod logging;
pub mod plugin;

pub use error::{PluginError, Result, WasmError, io_err};
pub use logging::{GUEST_TARGET, LogSink, tracing_sink};
pub use plugin::{Backend, Plugin, PluginExt, PluginInfo, Release, check_sdk, host_sdk_version, parse_version};
