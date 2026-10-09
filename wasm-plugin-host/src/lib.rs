// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A host for WebAssembly plugins, shared by applications that each define
//! their own plugin contract. It owns what every such host needs and nothing
//! about what plugins are for: loading a module with extism under a memory
//! cap, running each plugin on a worker thread of its own, cancelling a call
//! at its deadline, and serving calls the application is waiting on before
//! background events.
//!
//! The application supplies its host functions through [`HostEnv`] and names
//! its required export and limits in [`Options`].
//!
//! - [`host`] — [`PluginHost`]: load, the two lanes, deadlines, the memory cap.
//! - [`error`] — [`PluginError`], mapped from `extism::Error`.
//! - [`discover`] — user and project packages on disk, generic over the
//!   application's [`Manifest`].
//! - [`layout`] — stage a package, swap `current` / `previous`, `link`, and
//!   `remove` inside the plugins root.
//! - [`digest`] — [`package_digest`], the SHA-256 both of those agree on.

pub mod digest;
pub mod discover;
pub mod error;
pub mod host;
pub mod layout;

pub use digest::package_digest;
pub use discover::{Discovered, Manifest, Plugin, Source, State, load_package};
pub use error::PluginError;
/// Re-exported so host functions are built against the extism this crate runs.
pub use extism;
pub use host::{HostEnv, Lane, Limits, Options, PluginHost};
pub use layout::{
    CURRENT, LINK, PREVIOUS, activate, link, plugin_dir, read_pointer, remove, short, stage,
};
