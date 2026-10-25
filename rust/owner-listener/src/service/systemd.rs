//! Linux: a systemd user unit, enabled for the user's default target.

use super::{
    Backend, Installed, ServiceError, ServiceManager, Spec, Status, must, query, sync_file,
};

/// systemd --user backend. The unit name comes from [`Spec`]'s identity.
#[derive(Debug)]
pub struct Systemd;

/// A systemd word in double quotes.
///
/// `%` starts a specifier everywhere in a unit, so it is always doubled.
/// `$` starts an environment substitution in `ExecStart=`; it is doubled only
/// when `expand_dollar` is set. `Environment=` keeps `$` so a path is stored
/// as written.
pub(crate) fn quote(value: &str, expand_dollar: bool) -> String {
    let mut value = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    if expand_dollar {
        value = value.replace('$', "$$");
    }
    format!("\"{value}\"")
}

pub(crate) fn render(spec: &Spec) -> String {
    let mut argv = Vec::with_capacity(spec.args.len() + 1);
    argv.push(quote(&spec.exe, true));
    for arg in &spec.args {
        argv.push(quote(arg, true));
    }
    let exec = argv.join(" ");
    let env = if spec.home_pinned {
        let assignment = quote(&format!("{}={}", spec.identity.home_env, spec.home), false);
        format!("Environment={assignment}\n")
    } else {
        String::new()
    };
    // `append:` takes the rest of the line verbatim, so only `%` needs care.
    let log = spec.log_file().to_string_lossy().replace('%', "%%");
    format!(
        "[Unit]\n\
         Description={description}\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         {env}\
         Restart=on-failure\n\
         RestartSec=5\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        description = spec.identity.description.replace('%', "%%"),
    )
}

fn ctl(mgr: &mut dyn ServiceManager, unit: &str, args: &[&str]) -> Result<bool, ServiceError> {
    let mut all = vec!["--user"];
    all.extend_from_slice(args);
    all.push(unit);
    Ok(query(mgr, "systemctl", &all)?.success)
}

fn ctl_must(mgr: &mut dyn ServiceManager, unit: &str, args: &[&str]) -> Result<(), ServiceError> {
    let mut all = vec!["--user"];
    all.extend_from_slice(args);
    // daemon-reload takes no unit.
    if args != ["daemon-reload"] {
        all.push(unit);
    }
    must(mgr, "systemctl", &all).map(drop)
}

impl Backend for Systemd {
    fn install(
        &self,
        mgr: &mut dyn ServiceManager,
        spec: &Spec,
    ) -> Result<Installed, ServiceError> {
        let path = spec.unit_path();
        let unit = spec.identity.systemd_unit.as_str();
        let existed = mgr
            .read(&path)
            .map_err(|source| ServiceError::io("cannot read the unit", source))?
            .is_some();
        mgr.create_dir_all(&spec.log_dir())
            .map_err(|source| ServiceError::io("cannot create the log directory", source))?;
        let changed = sync_file(mgr, &path, &render(spec))?;
        let enabled = ctl(mgr, unit, &["is-enabled"])?;
        let active = ctl(mgr, unit, &["is-active"])?;
        if !changed && enabled && active {
            return Ok(Installed::Unchanged);
        }
        if changed {
            ctl_must(mgr, unit, &["daemon-reload"])?;
        }
        ctl_must(mgr, unit, &["enable", "--now"])?;
        // `enable --now` leaves a running service on its old definition.
        if changed && active {
            ctl_must(mgr, unit, &["restart"])?;
        }
        Ok(if existed {
            Installed::Updated
        } else {
            Installed::Fresh
        })
    }

    fn uninstall(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<bool, ServiceError> {
        let path = spec.unit_path();
        // Without the unit there is nothing to ask systemctl about, which also
        // keeps uninstall working on a host with no systemd.
        if mgr
            .read(&path)
            .map_err(|source| ServiceError::io("cannot read the unit", source))?
            .is_none()
        {
            return Ok(false);
        }
        let unit = spec.identity.systemd_unit.as_str();
        ctl_must(mgr, unit, &["disable", "--now"])?;
        mgr.remove(&path)
            .map_err(|source| ServiceError::io("cannot remove the unit", source))?;
        ctl_must(mgr, unit, &["daemon-reload"])?;
        Ok(true)
    }

    fn status(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<Status, ServiceError> {
        if mgr
            .read(&spec.unit_path())
            .map_err(|source| ServiceError::io("cannot read the unit", source))?
            .is_none()
        {
            return Ok(Status::NotInstalled);
        }
        let running = ctl(mgr, &spec.identity.systemd_unit, &["is-active"])?;
        Ok(Status::Installed { running })
    }
}
