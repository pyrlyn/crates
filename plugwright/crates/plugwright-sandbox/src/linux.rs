//! `no_new_privs` + Landlock (files) + seccomp-bpf (processes, sockets, signals).
//!
//! Both Landlock and seccomp apply to the calling thread and its future
//! children; the runner is single-threaded when it calls [`apply`].

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::{Report, Resolved};

// --- Landlock (uapi/linux/landlock.h) -------------------------------------

const CREATE_RULESET_VERSION: libc::c_uint = 1;
const RULE_PATH_BENEATH: libc::c_int = 1;

const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_V1: u64 = (1 << 13) - 1;
const FS_REFER: u64 = 1 << 13; // ABI 2
const FS_TRUNCATE: u64 = 1 << 14; // ABI 3
const FS_IOCTL_DEV: u64 = 1 << 15; // ABI 5
/// Rights that may be granted on a regular file (others need a directory).
const FS_FILE_RIGHTS: u64 = FS_EXECUTE | FS_WRITE_FILE | FS_READ_FILE | FS_TRUNCATE | FS_IOCTL_DEV;

const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0; // ABI 6
const SCOPE_SIGNAL: u64 = 1 << 1; // ABI 6

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

const SYSTEM_READ: &[&str] = &[
    "/usr",
    "/lib",
    "/lib32",
    "/lib64",
    "/nix/store",
    "/etc/ld.so.cache",
    "/etc/ld.so.conf",
    "/etc/ld.so.conf.d",
    "/etc/localtime",
    "/proc/self",
    "/sys/fs/cgroup",
    "/sys/devices/system/cpu",
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
];

fn landlock_abi() -> i64 {
    // SAFETY: documented probe call; no pointers dereferenced.
    unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    }
}

struct Fd(libc::c_int);

impl Drop for Fd {
    fn drop(&mut self) {
        // SAFETY: we own the descriptor.
        unsafe { libc::close(self.0) };
    }
}

fn open_path(p: &Path) -> Option<(Fd, bool)> {
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    // SAFETY: valid C string; O_PATH opens without reading.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
    Some((Fd(fd), is_dir))
}

fn add_rule(ruleset: &Fd, path: &Path, dir_rights: u64, handled: u64) -> Result<(), String> {
    let Some((fd, is_dir)) = open_path(path) else { return Ok(()) }; // absent paths are skipped
    let rights = if is_dir { dir_rights } else { dir_rights & FS_FILE_RIGHTS } & handled;
    let attr = PathBeneathAttr { allowed_access: rights, parent_fd: fd.0 };
    // SAFETY: `attr` is a valid landlock_path_beneath_attr for the duration of the call.
    let rc = unsafe { libc::syscall(libc::SYS_landlock_add_rule, ruleset.0, RULE_PATH_BENEATH, &attr, 0u32) };
    if rc < 0 {
        return Err(format!("landlock_add_rule({}): {}", path.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Returns the applied ABI version.
fn landlock(r: &Resolved) -> Result<i64, String> {
    let abi = landlock_abi();
    if abi < 1 {
        return Err(format!("Landlock unavailable: {}", std::io::Error::last_os_error()));
    }
    let mut handled = FS_V1;
    if abi >= 2 {
        handled |= FS_REFER;
    }
    if abi >= 3 {
        handled |= FS_TRUNCATE;
    }
    if abi >= 5 {
        handled |= FS_IOCTL_DEV;
    }
    let scoped = if abi >= 6 { SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL } else { 0 };
    let attr = RulesetAttr { handled_access_fs: handled, handled_access_net: 0, scoped };
    // Older kernels reject sizes they do not know; pass only the fields they understand.
    let size = if abi >= 6 { std::mem::size_of::<RulesetAttr>() } else { std::mem::size_of::<u64>() };
    // SAFETY: `attr` outlives the call and `size` does not exceed it.
    let fd = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, &attr, size, 0u32) };
    if fd < 0 {
        return Err(format!("landlock_create_ruleset: {}", std::io::Error::last_os_error()));
    }
    let ruleset = Fd(fd as libc::c_int);

    let read = FS_READ_FILE | FS_READ_DIR;
    for p in SYSTEM_READ {
        add_rule(&ruleset, Path::new(p), read, handled)?;
    }
    for p in &r.read {
        add_rule(&ruleset, p, read, handled)?;
    }
    add_rule(&ruleset, Path::new("/dev/null"), FS_READ_FILE | FS_WRITE_FILE, handled)?;
    let write = handled & !(FS_EXECUTE | FS_MAKE_CHAR | FS_MAKE_BLOCK);
    for p in &r.write {
        add_rule(&ruleset, p, write, handled)?;
    }
    // SAFETY: plain syscall on a descriptor we own; requires no_new_privs (set by caller).
    if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.0, 0u32) } < 0 {
        return Err(format!("landlock_restrict_self: {}", std::io::Error::last_os_error()));
    }
    Ok(abi)
}

// --- seccomp-bpf -------------------------------------------------------------

const BPF_LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
const BPF_JEQ_K: u16 = 0x05 | 0x10;
#[cfg(target_arch = "x86_64")]
const BPF_JGE_K: u16 = 0x05 | 0x30;
const BPF_JSET_K: u16 = 0x05 | 0x40;
const BPF_RET_K: u16 = 0x06;

const OFF_NR: u32 = 0;
const OFF_ARCH: u32 = 4;
const OFF_ARG0: u32 = 16; // low 32 bits on little-endian targets

const RET_ALLOW: u32 = 0x7fff_0000;
const RET_ERRNO: u32 = 0x0005_0000;

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xC000_003E;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xC000_00B7;

const CLONE_THREAD: u32 = 0x0001_0000;

fn stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt: 0, jf: 0, k }
}

fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

fn errno(e: i32) -> u32 {
    RET_ERRNO | (e as u32 & 0xffff)
}

/// Build the filter. Every rule ends in its own `RET`, so the accumulator
/// still holds the syscall number when a rule does not match.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter(network: bool, pid: u32) -> Vec<libc::sock_filter> {
    let mut f = vec![
        stmt(BPF_LD_W_ABS, OFF_ARCH),
        jump(BPF_JEQ_K, AUDIT_ARCH, 1, 0),
        stmt(BPF_RET_K, errno(libc::EPERM)),
        stmt(BPF_LD_W_ABS, OFF_NR),
    ];
    // x32 syscalls share AUDIT_ARCH_X86_64 but carry bit 30; refuse them all.
    #[cfg(target_arch = "x86_64")]
    f.extend([jump(BPF_JGE_K, 0x4000_0000, 0, 1), stmt(BPF_RET_K, errno(libc::EPERM))]);

    let mut deny: Vec<(libc::c_long, i32)> = vec![
        (libc::SYS_execve, libc::EACCES),
        (libc::SYS_execveat, libc::EACCES),
        (libc::SYS_ptrace, libc::EPERM),
        (libc::SYS_process_vm_readv, libc::EPERM),
        (libc::SYS_process_vm_writev, libc::EPERM),
        (libc::SYS_pidfd_getfd, libc::EPERM),
        (libc::SYS_io_uring_setup, libc::ENOSYS),
        (libc::SYS_io_uring_enter, libc::ENOSYS),
        (libc::SYS_io_uring_register, libc::ENOSYS),
        (libc::SYS_tkill, libc::EPERM),
        // glibc falls back to clone(2) on ENOSYS, whose flags we can inspect.
        (libc::SYS_clone3, libc::ENOSYS),
    ];
    #[cfg(target_arch = "x86_64")]
    deny.extend([(libc::SYS_fork, libc::EACCES), (libc::SYS_vfork, libc::EACCES)]);
    if !network {
        deny.push((libc::SYS_socket, libc::EACCES));
    }
    for (nr, e) in deny {
        f.push(jump(BPF_JEQ_K, nr as u32, 0, 1));
        f.push(stmt(BPF_RET_K, errno(e)));
    }
    // clone(2) is allowed only for threads.
    f.extend([
        jump(BPF_JEQ_K, libc::SYS_clone as u32, 0, 4),
        stmt(BPF_LD_W_ABS, OFF_ARG0),
        jump(BPF_JSET_K, CLONE_THREAD, 1, 0),
        stmt(BPF_RET_K, errno(libc::EACCES)),
        stmt(BPF_RET_K, RET_ALLOW),
    ]);
    // Signals only to ourselves.
    for nr in [libc::SYS_kill, libc::SYS_tgkill] {
        f.extend([
            jump(BPF_JEQ_K, nr as u32, 0, 4),
            stmt(BPF_LD_W_ABS, OFF_ARG0),
            jump(BPF_JEQ_K, pid, 1, 0),
            stmt(BPF_RET_K, errno(libc::EPERM)),
            stmt(BPF_RET_K, RET_ALLOW),
        ]);
    }
    f.push(stmt(BPF_RET_K, RET_ALLOW));
    f
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn seccomp(network: bool) -> Result<(), String> {
    let mut prog = filter(network, std::process::id());
    let fprog = libc::sock_fprog { len: prog.len() as u16, filter: prog.as_mut_ptr() };
    // SAFETY: `fprog` points to a valid filter that outlives the call; the kernel copies it.
    let rc = unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &fprog as *const _) };
    if rc != 0 {
        return Err(format!("seccomp: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn seccomp(_: bool) -> Result<(), String> {
    Err("seccomp filter not implemented for this architecture".into())
}

pub(super) fn capabilities() -> Report {
    let mut report = Report { applied: vec!["no_new_privs".into()], gaps: Vec::new() };
    match landlock_abi() {
        abi if abi >= 1 => report.applied.push(format!("landlock-v{abi}")),
        _ => report.gaps.push("file access (Landlock unavailable)".into()),
    }
    if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        report.applied.push("seccomp".into());
    } else {
        report.gaps.extend(["processes/signals".into(), "network".into()]);
    }
    report
}

pub(super) fn apply(r: &Resolved) -> Result<Report, String> {
    let mut report = Report::default();
    // SAFETY: plain prctl; required before an unprivileged seccomp filter or Landlock domain.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!("PR_SET_NO_NEW_PRIVS: {}", std::io::Error::last_os_error()));
    }
    report.applied.push("no_new_privs".into());
    match landlock(r) {
        Ok(abi) => report.applied.push(format!("landlock-v{abi}")),
        Err(e) => report.gaps.push(format!("file access ({e})")),
    }
    match seccomp(r.network) {
        Ok(()) => report.applied.push("seccomp".into()),
        Err(e) => {
            report.gaps.push(format!("processes/signals ({e})"));
            if !r.network {
                report.gaps.push("network".into());
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn filter_jumps_stay_in_bounds() {
        for network in [false, true] {
            let f = filter(network, 42);
            assert!(f.len() < 256);
            for (i, ins) in f.iter().enumerate() {
                if ins.code & 0x07 == 0x05 {
                    assert!(i + 1 + (ins.jt.max(ins.jf) as usize) < f.len(), "jump at {i} out of bounds");
                }
            }
            assert_eq!(f.last().unwrap().k, RET_ALLOW);
        }
    }
}
