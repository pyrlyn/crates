// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later

//! `tracing` setup an application would otherwise write by hand: JSON log files with daily
//! rotation, a human log on stderr and, behind the `otlp` feature, OTLP trace export. Secrets
//! are masked before any line reaches a sink and before any span leaves the process
//! (see [`redact`]).
//!
//! - [`Settings`], [`subscriber`] and [`init`] — the subscriber and its [`Guard`].
//! - [`redact`] — the masking rules, usable on their own.
//! - [`Scrubbed`] — the masking writer, for applications that build their own layers.

use std::path::PathBuf;

use tracing::Subscriber;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;
use tracing_subscriber::{EnvFilter, Layer, fmt};

#[cfg(feature = "otlp")]
mod otlp;
pub mod redact;
mod writer;

pub use redact::Redactor;
pub use writer::{ScrubWriter, Scrubbed};

/// Rotated files kept when the caller does not say otherwise: two weeks of daily logs.
pub const DEFAULT_MAX_FILES: usize = 14;

/// A layer boxed so the optional OTLP layer has the same type with and without the feature.
type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Why telemetry could not start.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The application name is empty or not a plain file name.
    #[error("invalid application name `{0}`: use letters, digits, `.`, `_` and `-`")]
    AppName(String),
    /// A redaction pattern does not compile.
    #[error("invalid redaction pattern `{pattern}`: {source}")]
    Pattern {
        /// The pattern as given.
        pattern: String,
        /// Why it does not compile.
        source: regex::Error,
    },
    /// The filter directive does not parse.
    #[error("invalid log filter `{directive}`: {source}")]
    Filter {
        /// The directive that was used: the override when set, else the level.
        directive: String,
        /// Why it does not parse.
        source: tracing_subscriber::filter::ParseError,
    },
    /// The log directory cannot be created or opened.
    #[error("cannot open the log directory: {0}")]
    LogDir(#[from] tracing_appender::rolling::InitError),
    /// [`init`] found a global subscriber already in place.
    #[error("a global tracing subscriber is already installed")]
    AlreadyInstalled(#[from] tracing::subscriber::SetGlobalDefaultError),
    /// An OTLP endpoint is set but the crate was built without the `otlp` feature.
    #[error("OTLP export is configured but this build lacks the `otlp` feature")]
    OtlpUnavailable,
    /// The OTLP exporter could not be built.
    #[cfg(feature = "otlp")]
    #[error("cannot start OTLP export: {0}")]
    Otlp(String),
}

/// Where and how to log.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Prefix of the log files (`<app>.<date>.log`) and the OTLP service name.
    pub app: String,
    /// Level from config (`info`, `my_store=debug,warn`, ...), used when `filter_override` is unset.
    pub level: String,
    /// Takes precedence over `level`; [`Settings::filter_from_env`] fills it.
    pub filter_override: Option<String>,
    /// Directory for the rotated JSON files; created when missing.
    pub log_dir: PathBuf,
    /// Rotated files to keep; values below 1 are treated as 1.
    pub max_files: usize,
    /// Also write a human-readable log to stderr (a CLI wants it, a daemon does not).
    pub stderr: bool,
    /// Full URL of an OTLP/HTTP traces endpoint, such as `http://localhost:4318/v1/traces`.
    /// Needs the `otlp` feature; plain HTTP only.
    pub otlp_endpoint: Option<String>,
    /// Credential shapes masked on top of the built-in ones, such as the application's own
    /// token prefix. See [`Redactor::new`].
    pub extra_secret_patterns: Vec<String>,
}

impl Settings {
    /// Settings with the stderr log on, [`DEFAULT_MAX_FILES`], no override and no OTLP.
    #[must_use]
    pub fn new(
        app: impl Into<String>,
        level: impl Into<String>,
        log_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            app: app.into(),
            level: level.into(),
            filter_override: None,
            log_dir: log_dir.into(),
            max_files: DEFAULT_MAX_FILES,
            stderr: true,
            otlp_endpoint: None,
            extra_secret_patterns: Vec::new(),
        }
    }

    /// Takes the filter override from the environment variable `var` (such as `MYAPP_LOG`)
    /// when it is set and not blank. It accepts any `EnvFilter` directive.
    #[must_use]
    pub fn filter_from_env(mut self, var: &str) -> Self {
        self.filter_override = std::env::var(var).ok().filter(|v| !v.trim().is_empty());
        self
    }

    fn filter(&self) -> Result<EnvFilter, Error> {
        let directive = self.filter_override.as_deref().unwrap_or(&self.level);
        EnvFilter::try_new(directive).map_err(|source| Error::Filter {
            directive: directive.to_owned(),
            source,
        })
    }

    // The name becomes a file name in `log_dir`; a separator or `..` would write elsewhere.
    fn checked_app(&self) -> Result<&str, Error> {
        let app = self.app.as_str();
        let plain = app
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if app.is_empty() || app.starts_with('.') || !plain {
            return Err(Error::AppName(app.to_owned()));
        }
        Ok(app)
    }
}

/// Keeps buffered logs and traces flowing; drop it (normally at the end of `main`) to flush them.
#[must_use = "dropping the guard stops log delivery"]
#[derive(Debug)]
pub struct Guard {
    _file: WorkerGuard,
    #[cfg(feature = "otlp")]
    _otlp: Option<otlp::Provider>,
}

/// Builds the subscriber without installing it, so tests and embedders can scope it with
/// `tracing::subscriber::with_default`.
///
/// # Errors
/// An invalid application name, redaction pattern or filter, an unusable log directory, or
/// OTLP requested without the feature.
pub fn subscriber(settings: &Settings) -> Result<(impl Subscriber + Send + Sync, Guard), Error> {
    let app = settings.checked_app()?;
    let redactor = Redactor::new(&settings.extra_secret_patterns)?;
    let filter = settings.filter()?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(app)
        .filename_suffix("log")
        .max_log_files(settings.max_files.max(1))
        .build(&settings.log_dir)?;
    // Non-blocking so a slow disk never stalls the application.
    let (file_writer, file_guard) = tracing_appender::non_blocking(appender);

    let file = fmt::layer()
        .json()
        .with_ansi(false)
        .with_writer(Scrubbed::new(file_writer, redactor.clone()));
    // No ANSI: escape codes around a field name would hide `token=` from the redaction patterns.
    let stderr = settings.stderr.then(|| {
        fmt::layer()
            .with_ansi(false)
            .with_writer(Scrubbed::new(std::io::stderr, redactor.clone()))
    });

    let (extra, guard) = extra_layer(settings, app, redactor, file_guard)?;
    let subscriber = Registry::default()
        .with(extra)
        .with(filter)
        .with(file)
        .with(stderr);
    Ok((subscriber, guard))
}

/// Installs the global subscriber and returns the guard to keep alive.
///
/// # Errors
/// Everything [`subscriber`] reports, and a subscriber that is already installed.
pub fn init(settings: &Settings) -> Result<Guard, Error> {
    let (subscriber, guard) = subscriber(settings)?;
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(guard)
}

#[cfg(feature = "otlp")]
fn extra_layer(
    settings: &Settings,
    app: &str,
    redactor: Redactor,
    file: WorkerGuard,
) -> Result<(Option<BoxedLayer>, Guard), Error> {
    let (layer, provider) = match settings.otlp_endpoint.as_deref() {
        Some(endpoint) => {
            let (layer, provider) = otlp::layer(endpoint, app, redactor)?;
            (Some(layer), Some(provider))
        }
        None => (None, None),
    };
    Ok((
        layer,
        Guard {
            _file: file,
            _otlp: provider,
        },
    ))
}

#[cfg(not(feature = "otlp"))]
fn extra_layer(
    settings: &Settings,
    _app: &str,
    _redactor: Redactor,
    file: WorkerGuard,
) -> Result<(Option<BoxedLayer>, Guard), Error> {
    if settings.otlp_endpoint.is_some() {
        return Err(Error::OtlpUnavailable);
    }
    Ok((None, Guard { _file: file }))
}
