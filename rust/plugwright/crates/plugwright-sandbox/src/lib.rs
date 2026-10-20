//! OS sandbox the isolated runner applies to itself before it loads a plugin.
//!
//! | platform | mechanism | enforced |
//! |---|---|---|
//! | macOS | Seatbelt (`sandbox_init`, SBPL profile) | reads allow-listed, no writes, no network, no exec/fork, no signals to others, no Mach lookups |
//! | Linux | `no_new_privs` + Landlock + seccomp-bpf | Landlock: reads/writes allow-listed, no exec; seccomp: no exec/fork, no sockets, no ptrace/`process_vm_*`, no io_uring, signals only to self |
//! | Windows | Low integrity token + job object | no writes to Medium-integrity locations, no child processes, UI limits; **reads and network are not restricted** |
//!
//! Whatever a platform cannot enforce is listed in [`Report::gaps`]. With
//! [`SandboxPolicy::require`] set, any gap makes the load fail instead.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Per-plugin sandbox policy (`[plugin.sandbox]` in the manifest).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxPolicy {
    /// Apply the sandbox at all. Only meaningful for `isolation = "process"`.
    pub enabled: bool,
    /// Allow network sockets.
    pub network: bool,
    /// Extra paths (files or directory trees) the plugin may read.
    pub read: Vec<PathBuf>,
    /// Paths (files or directory trees) the plugin may read and write.
    pub write: Vec<PathBuf>,
    /// Fail the load if the platform cannot enforce the whole policy.
    pub require: bool,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        SandboxPolicy { enabled: true, network: false, read: Vec::new(), write: Vec::new(), require: false }
    }
}

impl SandboxPolicy {
    pub fn disabled() -> Self {
        SandboxPolicy { enabled: false, ..Self::default() }
    }

    /// Resolve relative paths against `base` (the manifest directory).
    pub fn resolve(&mut self, base: &Path) {
        for p in self.read.iter_mut().chain(self.write.iter_mut()) {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
    }
}

/// What was actually enforced.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// Mechanisms applied, e.g. `["seatbelt"]` or `["no_new_privs", "landlock-v5", "seccomp"]`.
    pub applied: Vec<String>,
    /// Parts of the policy that could not be enforced on this system.
    pub gaps: Vec<String>,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.applied.is_empty() {
            f.write_str("none")?;
        } else {
            f.write_str(&self.applied.join("+"))?;
        }
        if !self.gaps.is_empty() {
            write!(f, " (not enforced: {})", self.gaps.join(", "))?;
        }
        Ok(())
    }
}

/// Confine the **current process**. Irreversible; call only in a runner.
///
/// `artifact` is the library about to be loaded; it is added to the read list.
pub fn apply(policy: &SandboxPolicy, artifact: &Path) -> Result<Report, String> {
    if !policy.enabled {
        return Ok(Report::default());
    }
    // Canonicalise while we still may: sandboxes match on real paths
    // (`/tmp` is `/private/tmp` on macOS).
    let canon = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
    let mut read: Vec<PathBuf> = policy.read.iter().map(canon).collect();
    read.push(canon(&artifact.to_path_buf()));
    if let Ok(exe) = std::env::current_exe() {
        read.push(canon(&exe));
    }
    let write: Vec<PathBuf> = policy.write.iter().map(canon).collect();
    let resolved = Resolved { network: policy.network, read, write };

    let report = imp::apply(&resolved)?;
    if policy.require && !report.gaps.is_empty() {
        return Err(format!("sandbox required but not fully enforceable here: {}", report.gaps.join(", ")));
    }
    Ok(report)
}

/// What [`apply`] would enforce here for the default policy, without
/// confining anything (for diagnostics).
pub fn capabilities() -> Report {
    imp::capabilities()
}

/// Policy with canonical absolute paths.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos", windows)), allow(dead_code))]
struct Resolved {
    network: bool,
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    pub(super) fn apply(_: &super::Resolved) -> Result<super::Report, String> {
        Ok(capabilities())
    }
    pub(super) fn capabilities() -> super::Report {
        super::Report { applied: Vec::new(), gaps: vec!["no sandbox on this OS".into()] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_parses_and_resolves() {
        let mut p: SandboxPolicy = toml::from_str("network = true\nread = [\"data\", \"/etc/hosts\"]").unwrap();
        assert!(p.enabled && p.network && !p.require);
        p.resolve(Path::new("/base"));
        assert_eq!(p.read, vec![PathBuf::from("/base/data"), PathBuf::from("/etc/hosts")]);
        assert!(toml::from_str::<SandboxPolicy>("bogus = 1").is_err());
    }

    #[test]
    fn report_display() {
        let r = Report { applied: vec!["a".into(), "b".into()], gaps: vec!["net".into()] };
        assert_eq!(r.to_string(), "a+b (not enforced: net)");
        assert_eq!(Report::default().to_string(), "none");
    }
}
