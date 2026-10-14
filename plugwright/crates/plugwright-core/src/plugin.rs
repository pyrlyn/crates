use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Result;

/// Which loader runs a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    /// Native library loaded into the host process.
    Native,
    /// Native library loaded into a child runner process.
    Isolated,
    /// Wasm module run by a Wasm engine (`plugwright-wasm`).
    Wasm,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Backend::Native => "native",
            Backend::Isolated => "isolated",
            Backend::Wasm => "wasm",
        })
    }
}

/// Metadata a plugin reports about itself when loaded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: semver::Version,
    pub sdk_version: semver::Version,
    pub backend: Backend,
    /// Engine name for Wasm plugins (`wasmtime`, `wasmi`, …).
    pub engine: Option<String>,
    /// OS sandbox enforced around an isolated plugin, e.g. `seatbelt` or
    /// `no_new_privs+landlock-v5+seccomp`; `None` when not applicable.
    pub sandbox: Option<String>,
}

/// What unloading could verify about a plugin's code leaving the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Release {
    /// The native library is no longer mapped (probed after `dlclose` /
    /// `FreeLibrary`).
    Unmapped,
    /// The library is still mapped after closing it: the loader keeps it alive
    /// (thread-local destructors, `RTLD_NODELETE`, a thread the plugin left
    /// running, …). Its code and statics stay in the process until exit; only
    /// process isolation guarantees a full unload.
    StillMapped,
    /// The runner process has exited, so the OS reclaimed everything.
    ProcessExited,
    /// Dropped with nothing left to verify (Wasm: the store and its memory).
    Freed,
}

impl Release {
    pub fn is_released(self) -> bool {
        self != Release::StillMapped
    }
}

impl std::fmt::Display for Release {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Release::Unmapped => "unmapped",
            Release::StillMapped => "still mapped",
            Release::ProcessExited => "process exited",
            Release::Freed => "freed",
        })
    }
}

/// The one interface every backend implements.
///
/// Inputs and outputs are bytes; [`PluginExt::call_json`] adds a typed layer.
/// Implementations never panic or abort on plugin misbehaviour — failures come
/// back as [`crate::PluginError`].
pub trait Plugin: Send {
    fn info(&self) -> &PluginInfo;
    fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>>;

    /// Destroy the plugin and report whether its code actually left the host.
    /// Dropping without calling this does the same work, minus the report.
    fn close(self: Box<Self>) -> Release {
        drop(self);
        Release::Freed
    }
}

/// Typed helpers over [`Plugin`].
pub trait PluginExt: Plugin {
    fn call_json<I: Serialize + ?Sized, O: DeserializeOwned>(&mut self, method: &str, input: &I) -> Result<O> {
        let bytes = serde_json::to_vec(input)?;
        let out = self.call(method, &bytes)?;
        Ok(serde_json::from_slice(&out)?)
    }
}

impl<P: Plugin + ?Sized> PluginExt for P {}

/// Parse `name\0version\0sdk_version`-style metadata strings into versions.
pub fn parse_version(what: &str, s: &str) -> Result<semver::Version> {
    semver::Version::parse(s).map_err(|e| crate::PluginError::Load(format!("invalid {what} `{s}`: {e}")))
}

/// Host-side SDK compatibility rule: same major (or same minor while 0.x).
pub fn check_sdk(id: &str, sdk: &semver::Version) -> Result<()> {
    let host = host_sdk_version();
    let req = semver::VersionReq::parse(&format!("^{}.{}", host.major, host.minor)).expect("valid req");
    // Match the release triple: `VersionReq` never matches pre-releases, which would
    // reject `0.2.0-alpha.1` (even from a host built at that very version).
    if req.matches(&semver::Version::new(sdk.major, sdk.minor, sdk.patch)) {
        Ok(())
    } else {
        Err(crate::PluginError::SdkMismatch { id: id.to_owned(), required: req.to_string(), found: sdk.to_string() })
    }
}

/// The SDK version this host was built against (same workspace version).
pub fn host_sdk_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("valid package version")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use semver::Version;

    use super::*;
    use crate::PluginError;

    #[test]
    fn backend_and_release_display() {
        assert_eq!(Backend::Native.to_string(), "native");
        assert_eq!(Backend::Isolated.to_string(), "isolated");
        assert_eq!(Backend::Wasm.to_string(), "wasm");
        assert_eq!(Release::Unmapped.to_string(), "unmapped");
        assert_eq!(Release::StillMapped.to_string(), "still mapped");
        assert_eq!(Release::ProcessExited.to_string(), "process exited");
        assert_eq!(Release::Freed.to_string(), "freed");
    }

    #[test]
    fn backend_and_release_serialize_kebab_case() {
        assert_eq!(serde_json::to_string(&Backend::Isolated).unwrap(), r#""isolated""#);
        assert_eq!(serde_json::from_str::<Backend>(r#""wasm""#).unwrap(), Backend::Wasm);
        assert!(serde_json::from_str::<Backend>(r#""Wasm""#).is_err());
        assert_eq!(serde_json::to_string(&Release::StillMapped).unwrap(), r#""still-mapped""#);
        assert_eq!(serde_json::to_string(&Release::ProcessExited).unwrap(), r#""process-exited""#);
    }

    #[test]
    fn only_still_mapped_is_not_released() {
        assert!(Release::Unmapped.is_released());
        assert!(Release::ProcessExited.is_released());
        assert!(Release::Freed.is_released());
        assert!(!Release::StillMapped.is_released());
    }

    #[test]
    fn parse_version_accepts_semver_and_names_the_field_on_error() {
        assert_eq!(parse_version("plugin version", "1.2.3-rc.1").unwrap(), Version::parse("1.2.3-rc.1").unwrap());
        for bad in ["1.2", "", "v1.0.0", "1.0.0 "] {
            match parse_version("SDK version", bad) {
                Err(PluginError::Load(msg)) => {
                    assert!(msg.starts_with(&format!("invalid SDK version `{bad}`: ")), "{msg}")
                }
                other => panic!("expected a load error for {bad:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn host_sdk_version_is_the_package_version() {
        assert_eq!(host_sdk_version(), Version::parse(env!("CARGO_PKG_VERSION")).unwrap());
    }

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version::new(major, minor, patch)
    }

    #[test]
    fn check_sdk_accepts_same_minor_while_0x_and_same_major_after() {
        let host = host_sdk_version();
        let mut ok = vec![host.clone(), v(host.major, host.minor, host.patch + 1), v(host.major, host.minor, 0)];
        let mut bad = vec![v(host.major + 1, 0, 0)];
        if host.major == 0 {
            bad.push(v(0, host.minor + 1, 0));
            if host.minor > 0 {
                bad.push(v(0, host.minor - 1, 99));
            }
        } else {
            ok.push(v(host.major, host.minor + 1, 0));
            bad.push(v(host.major - 1, 99, 0));
        }
        let mut with_build = host.clone();
        with_build.build = semver::BuildMetadata::new("git.abc").unwrap();
        ok.push(with_build);

        for sdk in ok {
            check_sdk("p", &sdk).unwrap_or_else(|e| panic!("{sdk} should be compatible: {e}"));
        }
        for sdk in bad {
            match check_sdk("p", &sdk) {
                Err(PluginError::SdkMismatch { id, required, found }) => {
                    assert_eq!(id, "p");
                    assert_eq!(required, format!("^{}.{}", host.major, host.minor));
                    assert_eq!(found, sdk.to_string());
                }
                other => panic!("{sdk} should be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn check_sdk_accepts_prereleases_of_a_compatible_version() {
        let host = host_sdk_version();
        for (patch, pre) in [(host.patch, "alpha.1"), (host.patch + 1, "rc.2")] {
            let mut sdk = v(host.major, host.minor, patch);
            sdk.pre = semver::Prerelease::new(pre).unwrap();
            check_sdk("p", &sdk).unwrap_or_else(|e| panic!("{sdk} should be compatible: {e}"));
        }
        let mut next = v(host.major + 1, 0, 0);
        next.pre = semver::Prerelease::new("alpha.1").unwrap();
        assert!(matches!(check_sdk("p", &next), Err(PluginError::SdkMismatch { .. })));
    }

    /// Echoes JSON back, returns garbage, or fails, by method name.
    struct Fake {
        info: PluginInfo,
        calls: Vec<(String, Vec<u8>)>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                info: PluginInfo {
                    name: "fake".into(),
                    version: v(1, 0, 0),
                    sdk_version: host_sdk_version(),
                    backend: Backend::Native,
                    engine: None,
                    sandbox: None,
                },
                calls: Vec::new(),
            }
        }
    }

    impl Plugin for Fake {
        fn info(&self) -> &PluginInfo {
            &self.info
        }

        fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
            self.calls.push((method.to_owned(), input.to_vec()));
            match method {
                "echo" => Ok(input.to_vec()),
                "garbage" => Ok(b"not json".to_vec()),
                _ => Err(PluginError::Call { method: method.into(), message: "unknown method".into() }),
            }
        }
    }

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Point {
        x: i32,
        label: String,
    }

    #[test]
    fn call_json_round_trips_through_bytes() {
        let mut p = Fake::new();
        let pt = Point { x: -3, label: "a\"b".into() };
        let back: Point = p.call_json("echo", &pt).unwrap();
        assert_eq!(back, pt);
        assert_eq!(p.calls, [("echo".to_owned(), serde_json::to_vec(&pt).unwrap())]);
    }

    #[test]
    fn call_json_works_on_trait_objects_and_unsized_inputs() {
        let mut p: Box<dyn Plugin> = Box::new(Fake::new());
        let out: Vec<u32> = p.call_json("echo", &[1u32, 2, 3][..]).unwrap();
        assert_eq!(out, [1, 2, 3]);
        let s: String = p.call_json("echo", "hi").unwrap();
        assert_eq!(s, "hi");
    }

    #[test]
    fn call_json_maps_bad_output_to_json_error() {
        let mut p = Fake::new();
        match p.call_json::<_, Point>("garbage", &1) {
            Err(PluginError::Json(e)) => assert!(e.is_syntax(), "{e}"),
            other => panic!("expected a JSON error, got {other:?}"),
        }
        match p.call_json::<_, Point>("echo", &1) {
            Err(PluginError::Json(e)) => assert!(e.is_data(), "{e}"),
            other => panic!("expected a JSON error, got {other:?}"),
        }
    }

    #[test]
    fn call_json_fails_before_calling_when_input_does_not_serialize() {
        let mut p = Fake::new();
        let input: HashMap<(i32, i32), i32> = HashMap::from([((1, 2), 3)]);
        assert!(matches!(p.call_json::<_, i32>("echo", &input), Err(PluginError::Json(_))));
        assert!(p.calls.is_empty());
    }

    #[test]
    fn call_json_passes_plugin_errors_through() {
        let mut p = Fake::new();
        match p.call_json::<_, i32>("missing", &()) {
            Err(PluginError::Call { method, message }) => {
                assert_eq!(method, "missing");
                assert_eq!(message, "unknown method");
            }
            other => panic!("expected a call error, got {other:?}"),
        }
    }

    #[test]
    fn default_close_reports_freed() {
        let p: Box<dyn Plugin> = Box::new(Fake::new());
        assert_eq!(p.info().name, "fake");
        assert_eq!(p.close(), Release::Freed);
    }
}
