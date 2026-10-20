//! macOS: a LaunchAgent plist, loaded into the user's GUI domain.

use super::{
    Backend, Installed, ServiceError, ServiceManager, Spec, Status, must, query, sync_file,
};

/// launchd backend. The label comes from [`Spec`]'s identity.
#[derive(Debug)]
pub struct Launchd;

fn escape_xml(value: &str) -> String {
    // `&` first, or the entities below would be escaped again.
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_string(value: &str) -> String {
    format!("<string>{}</string>", escape_xml(value))
}

pub(crate) fn render(spec: &Spec) -> String {
    let mut args = xml_string(&spec.exe);
    args.push('\n');
    for arg in &spec.args {
        args.push_str("    ");
        args.push_str(&xml_string(arg));
        args.push('\n');
    }
    let env = if spec.home_pinned {
        format!(
            "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>{}</key>\n    {}\n  </dict>\n",
            escape_xml(&spec.identity.home_env),
            xml_string(&spec.home)
        )
    } else {
        String::new()
    };
    let log = xml_string(&spec.log_file().to_string_lossy());
    let label = escape_xml(&spec.identity.launchd_label);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    {args}  </array>
{env}  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  {log}
  <key>StandardErrorPath</key>
  {log}
</dict>
</plist>
"#
    )
}

/// `gui/<uid>` is the user's login session, not a system-wide domain.
fn target(mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<(String, String), ServiceError> {
    let uid = must(mgr, "id", &["-u"])?;
    let domain = format!("gui/{}", uid.stdout.trim());
    let service = format!("{domain}/{}", spec.identity.launchd_label);
    Ok((domain, service))
}

fn loaded(mgr: &mut dyn ServiceManager, target: &str) -> Result<Option<String>, ServiceError> {
    let out = query(mgr, "launchctl", &["print", target])?;
    Ok(out.success.then_some(out.stdout))
}

impl Backend for Launchd {
    fn install(
        &self,
        mgr: &mut dyn ServiceManager,
        spec: &Spec,
    ) -> Result<Installed, ServiceError> {
        let path = spec.plist_path();
        let existed = mgr
            .read(&path)
            .map_err(|source| ServiceError::io("cannot read the plist", source))?
            .is_some();
        let (domain, target) = target(mgr, spec)?;
        let was_loaded = loaded(mgr, &target)?.is_some();
        mgr.create_dir_all(&spec.log_dir())
            .map_err(|source| ServiceError::io("cannot create the log directory", source))?;
        let changed = sync_file(mgr, &path, &render(spec))?;
        if !changed && was_loaded {
            return Ok(Installed::Unchanged);
        }
        if was_loaded {
            // A loaded job keeps its old definition until it is booted out.
            must(mgr, "launchctl", &["bootout", &target])?;
        }
        must(
            mgr,
            "launchctl",
            &["bootstrap", &domain, &path.to_string_lossy()],
        )?;
        Ok(if existed || was_loaded {
            Installed::Updated
        } else {
            Installed::Fresh
        })
    }

    fn uninstall(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<bool, ServiceError> {
        let path = spec.plist_path();
        if mgr
            .read(&path)
            .map_err(|source| ServiceError::io("cannot read the plist", source))?
            .is_none()
        {
            return Ok(false);
        }
        let (_, target) = target(mgr, spec)?;
        if loaded(mgr, &target)?.is_some() {
            must(mgr, "launchctl", &["bootout", &target])?;
        }
        mgr.remove(&path)
            .map_err(|source| ServiceError::io("cannot remove the plist", source))?;
        Ok(true)
    }

    fn status(&self, mgr: &mut dyn ServiceManager, spec: &Spec) -> Result<Status, ServiceError> {
        if mgr
            .read(&spec.plist_path())
            .map_err(|source| ServiceError::io("cannot read the plist", source))?
            .is_none()
        {
            return Ok(Status::NotInstalled);
        }
        let (_, target) = target(mgr, spec)?;
        let running = loaded(mgr, &target)?.is_some_and(|out| out.contains("state = running"));
        Ok(Status::Installed { running })
    }
}
