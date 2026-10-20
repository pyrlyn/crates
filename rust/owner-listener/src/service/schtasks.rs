//! Windows: a Task Scheduler task that runs at the user's logon.
//!
//! A task cannot redirect output without a shell, so the process keeps its
//! own log files. The home, when it is not the default, travels as a flag
//! because a task has no environment of its own.

use super::{Backend, Installed, ServiceError, ServiceManager, Spec, Status, must, query};

/// Task Scheduler backend. The task name comes from [`Spec`]'s identity.
#[derive(Debug)]
pub struct Schtasks;

/// One argument as the C runtime's command-line parser reads it back.
///
/// Backslashes only matter in front of a quote, and a trailing run of them
/// must be doubled before the closing quote.
pub(crate) fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_owned();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            other => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(other);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

pub(crate) fn command_line(spec: &Spec) -> String {
    let mut parts = Vec::with_capacity(spec.args.len() + 3);
    parts.push(quote(&spec.exe));
    if spec.home_pinned {
        parts.push(quote(&spec.identity.home_flag));
        parts.push(quote(&spec.home));
    }
    for arg in &spec.args {
        parts.push(quote(arg));
    }
    parts.join(" ")
}

fn exists(mgr: &mut dyn ServiceManager, task: &str) -> Result<bool, ServiceError> {
    Ok(query(mgr, "schtasks", &["/Query", "/TN", task])?.success)
}

impl Backend for Schtasks {
    fn install(
        &self,
        mgr: &mut dyn ServiceManager,
        spec: &Spec,
    ) -> Result<Installed, ServiceError> {
        let task = spec.identity.task_name.as_str();
        let existed = exists(mgr, task)?;
        let line = command_line(spec);
        // Task Scheduler cannot tell whether a definition changed, and `/F`
        // replaces it in place, so a repeat install rewrites it.
        must(
            mgr,
            "schtasks",
            &["/Create", "/SC", "ONLOGON", "/TN", task, "/TR", &line, "/F"],
        )?;
        if existed {
            return Ok(Installed::Updated);
        }
        // A logon trigger only fires at the next logon; start it now like the
        // other platforms do.
        must(mgr, "schtasks", &["/Run", "/TN", task])?;
        Ok(Installed::Fresh)
    }

    fn uninstall(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<bool, ServiceError> {
        let task = spec.identity.task_name.as_str();
        if !exists(mgr, task)? {
            return Ok(false);
        }
        // Ending a task that is not running fails, which is fine.
        query(mgr, "schtasks", &["/End", "/TN", task])?;
        must(mgr, "schtasks", &["/Delete", "/TN", task, "/F"])?;
        Ok(true)
    }

    fn status(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<Status, ServiceError> {
        let task = spec.identity.task_name.as_str();
        let out = query(mgr, "schtasks", &["/Query", "/TN", task, "/FO", "LIST"])?;
        if !out.success {
            return Ok(Status::NotInstalled);
        }
        // The state label is localised. The value `Running` is the English
        // name; other locales report "not running", which errs safe.
        let running = out
            .stdout
            .lines()
            .any(|line| line.trim_end().ends_with("Running"));
        Ok(Status::Installed { running })
    }
}
