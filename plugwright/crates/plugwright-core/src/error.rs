use std::path::PathBuf;

/// Every failure the manager can report. None of them unwinds into or aborts the host.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("manifest error: {0}")]
    Manifest(String),

    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("build of plugin `{id}` failed: {message}")]
    Build { id: String, message: String },

    #[error("git source error: {0}")]
    Git(String),

    #[error("{path} is not a plugwright plugin: {reason}")]
    NotAPlugin { path: PathBuf, reason: String },

    #[error("ABI mismatch: host speaks v{expected}, plugin speaks v{found}")]
    AbiMismatch { expected: u32, found: u32 },

    #[error("plugin `{id}`: SDK {found} is incompatible with host SDK requirement {required}")]
    SdkMismatch { id: String, required: String, found: String },

    #[error("plugin `{id}`: version {found} does not satisfy {required}")]
    VersionMismatch { id: String, required: String, found: String },

    #[error("integrity check failed for {path}: expected sha256 {expected}, got {actual}")]
    Integrity { path: PathBuf, expected: String, actual: String },

    #[error("failed to load plugin: {0}")]
    Load(String),

    /// The plugin ran and returned an error for this call.
    #[error("plugin returned an error from `{method}`: {message}")]
    Call { method: String, message: String },

    /// The plugin panicked; the panic was caught at the ABI boundary.
    #[error("plugin panicked: {0}")]
    Panicked(String),

    /// The isolated plugin process died (signal, abort, exit).
    #[error("plugin process crashed: {0}")]
    Crashed(String),

    #[error("plugin call timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("plugin `{0}` is disabled after too many crashes")]
    Disabled(String),

    #[error("protocol error talking to plugin runner: {0}")]
    Protocol(String),

    #[error(transparent)]
    Wasm(#[from] WasmError),

    #[error("no plugin with id `{0}`")]
    NotFound(String),

    #[error("plugin `{0}` is already loaded")]
    AlreadyLoaded(String),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl PluginError {
    /// A build failure of plugin `id` (empty when no plugin is involved yet).
    pub fn build(id: impl Into<String>, message: impl Into<String>) -> Self {
        PluginError::Build { id: id.into(), message: message.into() }
    }
}

/// Failures reported by engines and by the ABI glue.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WasmError {
    #[error("wasm compile error: {0}")]
    Compile(String),
    #[error("wasm instantiation error: {0}")]
    Instantiate(String),
    #[error("wasm module does not export `{0}`")]
    MissingExport(String),
    #[error("wasm trap: {0}")]
    Trap(String),
    #[error("wasm plugin ran out of fuel")]
    OutOfFuel,
    #[error("wasm plugin exceeded its time limit")]
    Timeout,
    #[error("wasm plugin exceeded its memory limit")]
    MemoryLimit,
    #[error("wasm ABI violation: {0}")]
    Abi(String),
    #[error("wasm memory access out of bounds")]
    OutOfBounds,
}

impl WasmError {
    /// After these the instance state cannot be trusted and is discarded.
    pub fn poisons_instance(&self) -> bool {
        !matches!(self, WasmError::Compile(_) | WasmError::Instantiate(_))
    }
}

pub type Result<T, E = PluginError> = std::result::Result<T, E>;

/// Map an I/O error to [`PluginError::Io`] carrying `path`.
pub fn io_err(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> PluginError {
    let path = path.into();
    move |source| PluginError::Io { path, source }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io;
    use std::time::Duration;

    use super::*;

    #[test]
    fn plugin_error_messages_include_their_fields() {
        let cases = [
            (PluginError::Manifest("bad key".into()), "manifest error: bad key"),
            (PluginError::build("hello", "linker failed"), "build of plugin `hello` failed: linker failed"),
            (PluginError::Git("no such ref".into()), "git source error: no such ref"),
            (
                PluginError::NotAPlugin { path: "/p/lib.so".into(), reason: "no entry symbol".into() },
                "/p/lib.so is not a plugwright plugin: no entry symbol",
            ),
            (PluginError::AbiMismatch { expected: 1, found: 7 }, "ABI mismatch: host speaks v1, plugin speaks v7"),
            (
                PluginError::SdkMismatch { id: "p".into(), required: "^0.1".into(), found: "0.2.0".into() },
                "plugin `p`: SDK 0.2.0 is incompatible with host SDK requirement ^0.1",
            ),
            (
                PluginError::VersionMismatch { id: "p".into(), required: ">=2".into(), found: "1.0.0".into() },
                "plugin `p`: version 1.0.0 does not satisfy >=2",
            ),
            (
                PluginError::Integrity { path: "/a.wasm".into(), expected: "ab".into(), actual: "cd".into() },
                "integrity check failed for /a.wasm: expected sha256 ab, got cd",
            ),
            (PluginError::Load("x".into()), "failed to load plugin: x"),
            (
                PluginError::Call { method: "greet".into(), message: "boom".into() },
                "plugin returned an error from `greet`: boom",
            ),
            (PluginError::Panicked("oops".into()), "plugin panicked: oops"),
            (PluginError::Crashed("SIGSEGV".into()), "plugin process crashed: SIGSEGV"),
            (PluginError::Timeout(Duration::from_millis(1500)), "plugin call timed out after 1.5s"),
            (PluginError::Disabled("p".into()), "plugin `p` is disabled after too many crashes"),
            (PluginError::Protocol("short frame".into()), "protocol error talking to plugin runner: short frame"),
            (PluginError::NotFound("p".into()), "no plugin with id `p`"),
            (PluginError::AlreadyLoaded("p".into()), "plugin `p` is already loaded"),
            (PluginError::Unsupported("wasm on this target".into()), "unsupported: wasm on this target"),
        ];
        for (err, msg) in cases {
            assert_eq!(err.to_string(), msg);
        }
    }

    #[test]
    fn wasm_error_messages() {
        let cases = [
            (WasmError::Compile("bad magic".into()), "wasm compile error: bad magic"),
            (WasmError::Instantiate("import denied".into()), "wasm instantiation error: import denied"),
            (WasmError::MissingExport("pw_call".into()), "wasm module does not export `pw_call`"),
            (WasmError::Trap("unreachable".into()), "wasm trap: unreachable"),
            (WasmError::OutOfFuel, "wasm plugin ran out of fuel"),
            (WasmError::Timeout, "wasm plugin exceeded its time limit"),
            (WasmError::MemoryLimit, "wasm plugin exceeded its memory limit"),
            (WasmError::Abi("empty result buffer".into()), "wasm ABI violation: empty result buffer"),
            (WasmError::OutOfBounds, "wasm memory access out of bounds"),
        ];
        for (err, msg) in cases {
            assert_eq!(err.to_string(), msg);
            // `PluginError::Wasm` is transparent.
            assert_eq!(PluginError::from(err).to_string(), msg);
        }
    }

    #[test]
    fn only_compile_and_instantiate_errors_leave_the_instance_usable() {
        assert!(!WasmError::Compile(String::new()).poisons_instance());
        assert!(!WasmError::Instantiate(String::new()).poisons_instance());
        for e in [
            WasmError::MissingExport(String::new()),
            WasmError::Trap(String::new()),
            WasmError::OutOfFuel,
            WasmError::Timeout,
            WasmError::MemoryLimit,
            WasmError::Abi(String::new()),
            WasmError::OutOfBounds,
        ] {
            assert!(e.poisons_instance(), "{e:?}");
        }
    }

    #[test]
    fn build_accepts_an_empty_id() {
        match PluginError::build("", String::from("cargo not found")) {
            PluginError::Build { id, message } => {
                assert_eq!(id, "");
                assert_eq!(message, "cargo not found");
            }
            other => panic!("expected a build error, got {other:?}"),
        }
    }

    #[test]
    fn io_err_keeps_path_and_source() {
        let err = io_err("/tmp/plugwright.toml")(io::Error::new(io::ErrorKind::NotFound, "gone"));
        assert_eq!(err.to_string(), "i/o error at /tmp/plugwright.toml: gone");
        let source = err.source().and_then(|s| s.downcast_ref::<io::Error>()).expect("io source");
        assert_eq!(source.kind(), io::ErrorKind::NotFound);
        match err {
            PluginError::Io { path, .. } => assert_eq!(path, PathBuf::from("/tmp/plugwright.toml")),
            other => panic!("expected an i/o error, got {other:?}"),
        }
    }

    #[test]
    fn json_errors_convert_and_keep_serde_message() {
        let json = serde_json::from_str::<u32>("nope").unwrap_err();
        let expected = format!("JSON error: {json}");
        assert_eq!(PluginError::from(json).to_string(), expected);
    }
}
