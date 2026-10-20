//! Windows security descriptor and process-token helpers.
//!
//! Every unsafe call is a Win32 API that has no safe wrapper in `std`. Each
//! block states what the pointer refers to and how long it stays valid.
//! The rest of the crate keeps `unsafe_code` denied.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io::{self, ErrorKind};
use std::os::windows::io::AsRawHandle;
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::sddl::user_only_dacl;

/// A user SID this crate has already checked against [`user_only_dacl`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UserSid(String);

impl UserSid {
    pub(crate) fn current() -> io::Result<Self> {
        let mut token = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle that must not be
        // closed. OpenProcessToken writes a real handle into `token` on success.
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if opened == 0 {
            return Err(last_error());
        }
        let token = OwnedHandle(token);
        Self::from_token(token.0)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn parse(text: String) -> io::Result<Self> {
        if !crate::sddl::is_sid(&text) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "token sid was not a SID",
            ));
        }
        Ok(Self(text))
    }

    fn from_token(token: HANDLE) -> io::Result<Self> {
        Self::parse(token_user_string(token)?)
    }
}

/// Creates one pipe instance. `first` is set only for the instance that claims
/// the name; later instances of the same pipe must leave it unset or Windows
/// refuses them.
pub(crate) fn create_server(name: &str, sid: &UserSid, first: bool) -> io::Result<NamedPipeServer> {
    let sddl = user_only_dacl(sid.as_str())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "refusing to build a user DACL"))?;
    let descriptor = SecurityDescriptor::from_sddl(&sddl)?;
    let mut attrs = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>())
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "security attributes size"))?,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first);
    options.reject_remote_clients(true);
    // SAFETY: `attrs` points at a SECURITY_ATTRIBUTES on this stack frame.
    // `descriptor` owns the security descriptor and is not freed until this
    // function returns. CreateNamedPipe copies the descriptor before it
    // returns, which is the contract of create_with_security_attributes_raw.
    unsafe {
        options.create_with_security_attributes_raw(name, ptr::addr_of_mut!(attrs).cast::<c_void>())
    }
}

/// Process id of the client connected to a server pipe instance.
pub(crate) fn client_process_id(pipe: &impl AsRawHandle) -> io::Result<u32> {
    process_id(pipe, true)
}

/// Process id of the server at the other end of a client pipe handle.
pub(crate) fn server_process_id(pipe: &impl AsRawHandle) -> io::Result<u32> {
    process_id(pipe, false)
}

/// User SID of `pid`, or an error when the process cannot be queried.
/// Failure refuses the client: a peer whose user is unknown is not the owner.
pub(crate) fn process_user_sid(pid: u32) -> io::Result<UserSid> {
    // SAFETY: OpenProcess returns a handle we take ownership of, or null.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(last_error());
    }
    let process = OwnedHandle(process);
    let mut token = ptr::null_mut();
    // SAFETY: `process` is an open process handle. On success `token` is a
    // handle this function owns.
    let opened = unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(last_error());
    }
    let token = OwnedHandle(token);
    UserSid::from_token(token.0)
}

fn process_id(pipe: &impl AsRawHandle, client: bool) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: the pipe handle is open for the duration of the call. `pid`
    // is a local the API writes.
    let ok = unsafe {
        let handle = pipe.as_raw_handle() as HANDLE;
        if client {
            GetNamedPipeClientProcessId(handle, &mut pid)
        } else {
            GetNamedPipeServerProcessId(handle, &mut pid)
        }
    };
    if ok == 0 || pid == 0 {
        return Err(if ok == 0 {
            last_error()
        } else {
            io::Error::new(ErrorKind::InvalidData, "pipe process id was 0")
        });
    }
    Ok(pid)
}

fn token_user_string(token: HANDLE) -> io::Result<String> {
    let mut len = 0u32;
    // SAFETY: a null buffer with a zero length is the documented way to ask
    // for the size. The token handle stays open.
    let probed = unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut len) };
    if len == 0 || len > 65_536 {
        return Err(if probed == 0 {
            last_error()
        } else {
            io::Error::new(ErrorKind::InvalidData, "token user size was not usable")
        });
    }
    let words = len.div_ceil(8);
    let mut buf = vec![0u64; usize::try_from(words).unwrap_or(0)];
    if buf.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "token user size was not usable",
        ));
    }
    // SAFETY: `buf` is at least `len` bytes and aligned to 8, which covers
    // TOKEN_USER. The sid pointer below is only used before `buf` is dropped.
    let read =
        unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) };
    if read == 0 {
        return Err(last_error());
    }
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    if sid.is_null() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "token user had no sid",
        ));
    }
    sid_to_string(sid)
}

fn sid_to_string(sid: *mut c_void) -> io::Result<String> {
    let mut wide = ptr::null_mut();
    // SAFETY: `sid` points into the live token buffer. On success `wide` is
    // a LocalAlloc string that this function frees.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut wide) };
    if ok == 0 || wide.is_null() {
        return Err(last_error());
    }
    let text = read_wide(wide);
    // SAFETY: `wide` came from ConvertSidToStringSidW and has not been freed.
    unsafe {
        LocalFree(wide.cast::<c_void>());
    }
    text
}

fn read_wide(wide: *const u16) -> io::Result<String> {
    let mut len = 0usize;
    // SAFETY: `wide` is a null-terminated LocalAlloc string. The scan stops
    // at 256 code units, longer than any SID string this crate accepts.
    unsafe {
        while len < 256 && *wide.add(len) != 0 {
            len += 1;
        }
        if len == 0 || len == 256 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "sid string was empty or not terminated",
            ));
        }
        let slice = std::slice::from_raw_parts(wide, len);
        String::from_utf16(slice)
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "sid string was not utf-16"))
    }
}

struct SecurityDescriptor(*mut c_void);

impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let mut wide: Vec<u16> = sddl.encode_utf16().collect();
        wide.push(0);
        let mut descriptor = ptr::null_mut();
        // SAFETY: `wide` is a null-terminated UTF-16 SDDL string that lives
        // for this call. On success `descriptor` is a LocalAlloc block this
        // value owns.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(last_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer was allocated by
            // ConvertStringSecurityDescriptorToSecurityDescriptorW and has
            // not been freed.
            unsafe {
                LocalFree(self.0.cast::<c_void>());
            }
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle was opened by this module and is not used again.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn last_error() -> io::Error {
    // SAFETY: GetLastError reads the calling thread's last-error code.
    let code = unsafe { GetLastError() };
    io::Error::from_raw_os_error(i32::try_from(code).unwrap_or(i32::MAX))
}
