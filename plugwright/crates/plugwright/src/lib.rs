#![doc = include_str!("../README.md")]

pub mod client;
pub mod manager;
pub mod manifest;
pub mod watch;

pub use plugwright_build::{accel, builder, cache, git, package, relink};
pub use plugwright_core::{error, logging, plugin};
pub use plugwright_isolate as isolate;
pub use plugwright_loader as loader;
#[cfg(feature = "wasmi")]
pub use plugwright_loader::WasmiEngine;
#[cfg(feature = "wasmtime")]
pub use plugwright_loader::WasmtimeEngine;
pub use plugwright_loader::{Loader, NativeLoader, WasmLoader, builtin_engines};
pub use plugwright_native as native;
pub use plugwright_sandbox as sandbox;
pub use plugwright_wasm as wasm;

pub use builder::{BuildOptions, BuildRequest, Builder, CrateType, Target};
pub use cache::{Artifact, Cache};
#[doc(hidden)]
pub use client::__private;
pub use client::{Bound, Invoke, MethodInfo};
pub use error::{PluginError, Result};
pub use isolate::{IsolatedPlugin, IsolationOptions, RunnerCommand, maybe_run_runner};
pub use manager::{ManagerOptions, PluginManager};
pub use manifest::{Isolation, Lockfile, Manifest, PluginKind, PluginSpec, Source};
pub use native::{NativePlugin, Placement};
pub use plugin::{Backend, Plugin, PluginExt, PluginInfo, Release};
pub use plugwright_macros::interface;
pub use sandbox::SandboxPolicy;
pub use wasm::{ModuleCache, WasmEngine, WasmError, WasmInstance, WasmLimits, WasmModule, WasmPlugin, WasmVal};
pub use watch::{Reloader, WatchOptions, Watcher};
