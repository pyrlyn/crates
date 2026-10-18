//! Render [D2](https://d2lang.com) diagrams from Rust.
//!
//! By default diagrams are rendered in-process by a pure-Rust port of a
//! subset of D2 ([`NativeBackend`], feature `native`): no `d2` binary, no
//! subprocess. Enable feature `cli` to render through the `d2` executable
//! instead ([`Renderer::cli`]), which supports the whole language, every
//! layout engine and every output format. Either way stderr-style messages
//! become structured [`Diagnostic`]s, a successful render returns a typed
//! [`RenderReport`], and rendered SVG is scanned for its size and the keys
//! of every shape and connection ([`SvgInfo`]).
//!
//! ```no_run
//! use d2_render::{Format, Renderer, RenderOptions};
//! use std::path::Path;
//!
//! let renderer = Renderer::new().options(RenderOptions::new().theme(200).pad(20));
//! let out = renderer.render_str("a -> b", Path::new("out/ab.svg"), Format::Svg)?;
//! println!("wrote {}", out.display());
//! # Ok::<(), d2_render::Error>(())
//! ```
//!
//! Features: `native` (default), `cli` (the `d2` executable backend), `png`
//! (PNG from the native backend via `resvg`), `tui` (ratatui widget), `web`
//! (HTML embedding helpers) and `watch` (debounced re-render on change).
//! See the README for each.

#![cfg_attr(docsrs, feature(doc_cfg))]

mod backend;
#[cfg(feature = "cli")]
mod cli;
pub mod diagnostic;
mod error;
pub mod fresh;
mod options;
mod renderer;
mod report;
pub mod svg;

#[cfg(feature = "native")]
pub mod native;
#[cfg(feature = "tui")]
pub mod tui;
#[cfg(feature = "watch")]
pub mod watch;
#[cfg(feature = "web")]
pub mod web;

pub use backend::{Backend, BackendOutput};
#[cfg(feature = "cli")]
pub use cli::{CliBackend, D2_BIN_ENV};
pub use diagnostic::{parse_stderr, Diagnostic, ParsedOutput, Severity};
pub use error::{Error, Result};
pub use fresh::{Freshness, StaleOutput};
#[cfg(feature = "native")]
pub use native::NativeBackend;
pub use options::{Format, Layout, RenderOptions};
pub use renderer::Renderer;
pub use report::RenderReport;
pub use svg::{ElementKind, SvgElement, SvgInfo};
