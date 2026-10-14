//! Low integrity level + job object. Reads and network stay open: closing them
//! needs an AppContainer, which must be set up by the *parent* at
//! `CreateProcess` time (future work, see plan F2).

use std::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel, WinLowLabelSid,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS,
    JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
    JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{Report, Resolved};

/// `SE_GROUP_INTEGRITY` (winnt.h); lives in a feature we do not otherwise need.
const SE_GROUP_INTEGRITY: u32 = 0x20;

fn last_error(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn low_integrity() -> Result<(), String> {
    // SAFETY: Win32 calls with valid out-pointers; the token handle is closed below.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, &mut token) == 0 {
            return Err(last_error("OpenProcessToken"));
        }
        let mut sid = [0u8; 68]; // SECURITY_MAX_SID_SIZE
        let mut sid_len = sid.len() as u32;
        if CreateWellKnownSid(WinLowLabelSid, std::ptr::null_mut(), sid.as_mut_ptr().cast(), &mut sid_len) == 0 {
            CloseHandle(token);
            return Err(last_error("CreateWellKnownSid"));
        }
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES { Sid: sid.as_mut_ptr().cast(), Attributes: SE_GROUP_INTEGRITY },
        };
        let ok = SetTokenInformation(
            token,
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast::<c_void>(),
            size_of::<TOKEN_MANDATORY_LABEL>() as u32 + sid_len,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(last_error("SetTokenInformation(TokenIntegrityLevel)"));
        }
    }
    Ok(())
}

fn job() -> Result<(), String> {
    // SAFETY: Win32 calls with properly sized, zero-initialised structures. The
    // job handle is intentionally kept open for the life of the process.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(limits)"));
        }
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        };
        if SetInformationJobObject(
            job,
            JobObjectBasicUIRestrictions,
            (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject(ui)"));
        }
        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            return Err(last_error("AssignProcessToJobObject"));
        }
    }
    Ok(())
}

pub(super) fn capabilities() -> Report {
    Report { applied: vec!["low-integrity".into(), "job-object".into()], gaps: vec!["reads".into(), "network".into()] }
}

pub(super) fn apply(r: &Resolved) -> Result<Report, String> {
    let mut report = Report::default();
    match low_integrity() {
        Ok(()) => report.applied.push("low-integrity".into()),
        Err(e) => report.gaps.push(format!("writes ({e})")),
    }
    match job() {
        Ok(()) => report.applied.push("job-object".into()),
        Err(e) => report.gaps.push(format!("processes ({e})")),
    }
    // Every file stays readable, so a read allow-list has nothing to narrow.
    report.gaps.push(format!("reads ({} allow-listed paths, everything else readable too)", r.read.len()));
    if !r.network {
        report.gaps.push("network".into());
    }
    if !r.write.is_empty() {
        report.gaps.push("write allow-list (low integrity denies all medium-integrity paths)".into());
    }
    Ok(report)
}
