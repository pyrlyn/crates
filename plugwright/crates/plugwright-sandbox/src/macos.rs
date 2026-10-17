//! Seatbelt via `sandbox_init`. The API is marked deprecated but is what every
//! sandboxed macOS app (Chrome, Firefox, Safari helpers) still uses.

use std::ffi::{CStr, CString, c_char, c_int};
use std::fmt::Write as _;
use std::path::Path;

use super::{Report, Resolved};

unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// System locations dyld and libSystem read at run time.
const SYSTEM_READ: &[&str] = &[
    "/System",
    "/usr/lib",
    "/usr/share",
    "/Library/Apple",
    "/private/var/db/dyld",
    "/private/var/db/timezone",
    "/private/etc/localtime",
    "/dev/null",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
];

fn quote(p: &Path) -> Result<String, String> {
    let s = p.to_str().ok_or_else(|| format!("sandbox path is not UTF-8: {p:?}"))?;
    Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// Build the SBPL profile. Later rules take precedence over earlier ones.
pub(super) fn profile(r: &Resolved) -> Result<String, String> {
    let mut p = String::from("(version 1)\n(allow default)\n");
    // Files: metadata stays readable (path resolution), content is allow-listed.
    p.push_str("(deny file-read-data)\n(deny file-read-xattr)\n(deny file-write*)\n");
    p.push_str("(allow file-read-data file-read-xattr (literal \"/\")");
    for s in SYSTEM_READ {
        let _ = write!(p, " (subpath {})", quote(Path::new(s))?);
    }
    for path in &r.read {
        let _ = write!(p, " (subpath {})", quote(path)?);
    }
    p.push_str(")\n");
    p.push_str("(allow file-write* (literal \"/dev/null\"))\n");
    for path in &r.write {
        let q = quote(path)?;
        let _ = writeln!(p, "(allow file-read* file-write* (subpath {q}))");
    }
    // Processes: no exec, no fork, signals only to ourselves.
    p.push_str("(deny process-exec*)\n(deny process-fork)\n");
    p.push_str("(deny signal)\n(allow signal (target self))\n");
    p.push_str("(deny process-info* (target others))\n");
    // IPC: no Mach services (launchd, LaunchServices, pasteboard, …).
    p.push_str("(deny mach-lookup)\n(deny mach-register)\n");
    if !r.network {
        p.push_str("(deny network*)\n");
    }
    Ok(p)
}

pub(super) fn capabilities() -> Report {
    Report { applied: vec!["seatbelt".into()], gaps: Vec::new() }
}

pub(super) fn apply(r: &Resolved) -> Result<Report, String> {
    let profile = CString::new(profile(r)?).map_err(|_| "sandbox profile contains NUL".to_owned())?;
    let mut err: *mut c_char = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated profile; `err` is freed with the matching API.
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut err) };
    if rc != 0 {
        let msg = if err.is_null() {
            "unknown error".to_owned()
        } else {
            // SAFETY: on failure `err` points to a C string owned by libsandbox.
            let m = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
            unsafe { sandbox_free_error(err) };
            m
        };
        return Err(format!("sandbox_init failed: {msg}"));
    }
    Ok(Report { applied: vec!["seatbelt".into()], gaps: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn profile_quotes_and_lists_paths() {
        let r = Resolved { network: false, read: vec![PathBuf::from("/a \"b\"")], write: vec![PathBuf::from("/w")] };
        let p = profile(&r).unwrap();
        assert!(p.contains(r#"(subpath "/a \"b\"")"#));
        assert!(p.contains(r#"(allow file-read* file-write* (subpath "/w"))"#));
        assert!(p.contains("(deny network*)"));
        let open = Resolved { network: true, read: vec![], write: vec![] };
        assert!(!profile(&open).unwrap().contains("network"));
    }
}
