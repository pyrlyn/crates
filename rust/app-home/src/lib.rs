//! Resolve where a command-line app keeps its files, the same way on every OS.
//!
//! ```no_run
//! let home = app_home::app_home("AULO_HOME", ".aulo");
//! let cfg = app_home::config_home().map(|d| d.join("runa"));
//! let bin = app_home::Dirs::from_env().expand_tilde("~\\.ketch\\bin");
//! ```
//!
//! Every resolver also runs over an injected variable lookup ([`Dirs::with`]),
//! so tests never read or mutate the process environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The user home from the process environment. See [`Dirs::user_home`].
pub fn user_home() -> Option<PathBuf> {
    Dirs::from_env().user_home()
}

/// An app's home from the process environment. See [`Dirs::app_home`].
pub fn app_home(var: &str, dot_name: &str) -> Option<PathBuf> {
    Dirs::from_env().app_home(var, dot_name)
}

/// `$XDG_CONFIG_HOME`, else `~/.config`. See [`Dirs::config_home`].
pub fn config_home() -> Option<PathBuf> {
    Dirs::from_env().config_home()
}

/// `$XDG_DATA_HOME`, else `~/.local/share`. See [`Dirs::config_home`].
pub fn data_home() -> Option<PathBuf> {
    Dirs::from_env().data_home()
}

/// `$XDG_CACHE_HOME`, else `~/.cache`. See [`Dirs::config_home`].
pub fn cache_home() -> Option<PathBuf> {
    Dirs::from_env().cache_home()
}

/// `$XDG_STATE_HOME`, else `~/.local/state`. See [`Dirs::config_home`].
pub fn state_home() -> Option<PathBuf> {
    Dirs::from_env().state_home()
}

/// `~`, `~/rest` and `~\rest` against `home`; anything else unchanged.
///
/// The rest is joined by components on `/` and `\`: PowerShell and Windows
/// configs write `~\.app\bin`, and `home.join(".app\\bin")` would be one
/// literal file name on Unix. `~user` forms are left alone.
pub fn expand_tilde(path: impl AsRef<Path>, home: &Path) -> PathBuf {
    let path = path.as_ref();
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if text == "~" {
        return home.to_path_buf();
    }
    match text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        Some(rest) => {
            let mut out = home.to_path_buf();
            out.extend(rest.split(['/', '\\']).filter(|s| !s.is_empty()));
            out
        }
        None => path.to_path_buf(),
    }
}

/// The resolution rules over one variable lookup.
pub struct Dirs<F> {
    var: F,
    os_fallback: bool,
}

impl Dirs<fn(&str) -> Option<OsString>> {
    /// Over the process environment, falling back to the OS account's home
    /// when neither `HOME` nor `USERPROFILE` is set.
    pub fn from_env() -> Self {
        Dirs {
            var: |key| std::env::var_os(key),
            os_fallback: true,
        }
    }
}

impl<F: Fn(&str) -> Option<OsString>> Dirs<F> {
    /// Over `var` alone, with no OS lookup, so a test's answer depends only
    /// on the variables it hands in.
    pub fn with(var: F) -> Self {
        Dirs {
            var,
            os_fallback: false,
        }
    }

    /// A set, non-empty variable. Native Windows shells often export an empty
    /// `HOME`, and an empty value must not block the next fallback.
    fn nonempty(&self, key: &str) -> Option<PathBuf> {
        (self.var)(key).filter(|v| !v.is_empty()).map(PathBuf::from)
    }

    /// `HOME`, else `USERPROFILE`, else (for [`Dirs::from_env`]) the OS
    /// account's home. `HOME` comes first on Windows too: Git Bash, MSYS and
    /// test harnesses set it to move the home, and std ignores it there.
    pub fn user_home(&self) -> Option<PathBuf> {
        self.nonempty("HOME")
            .or_else(|| self.nonempty("USERPROFILE"))
            .or_else(|| if self.os_fallback { os_home() } else { None })
    }

    /// `$var` with a leading `~` expanded, else `<user home>/<dot_name>`.
    ///
    /// The `~` is expanded because a value from a JSON `env` block or a
    /// service unit is never shell-expanded, and a literal `~` would hang the
    /// whole tree off `./~` in the current directory. A relative `$var` is
    /// returned as given; callers that must not depend on the current
    /// directory check `is_absolute`.
    pub fn app_home(&self, var: &str, dot_name: &str) -> Option<PathBuf> {
        match self.nonempty(var) {
            Some(dir) => Some(self.expand_tilde(dir)),
            None => self.user_home().map(|h| h.join(dot_name)),
        }
    }

    /// `$XDG_CONFIG_HOME` when absolute, else `<user home>/.config`.
    ///
    /// The XDG spec calls a relative value invalid, so it is ignored rather
    /// than resolved against whatever the current directory happens to be.
    /// The `~/.config` fallback applies on macOS and Windows too: command-line
    /// tools keep the same layout everywhere instead of `Library` or
    /// `AppData`.
    pub fn config_home(&self) -> Option<PathBuf> {
        self.xdg("XDG_CONFIG_HOME", &[".config"])
    }

    /// `$XDG_DATA_HOME` when absolute, else `<user home>/.local/share`.
    pub fn data_home(&self) -> Option<PathBuf> {
        self.xdg("XDG_DATA_HOME", &[".local", "share"])
    }

    /// `$XDG_CACHE_HOME` when absolute, else `<user home>/.cache`.
    pub fn cache_home(&self) -> Option<PathBuf> {
        self.xdg("XDG_CACHE_HOME", &[".cache"])
    }

    /// `$XDG_STATE_HOME` when absolute, else `<user home>/.local/state`.
    pub fn state_home(&self) -> Option<PathBuf> {
        self.xdg("XDG_STATE_HOME", &[".local", "state"])
    }

    /// [`expand_tilde`] against this lookup's user home; unchanged when there
    /// is none.
    pub fn expand_tilde(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if !path.to_string_lossy().starts_with('~') {
            return path.to_path_buf();
        }
        match self.user_home() {
            Some(home) => expand_tilde(path, &home),
            None => path.to_path_buf(),
        }
    }

    fn xdg(&self, key: &str, fallback: &[&str]) -> Option<PathBuf> {
        if let Some(dir) = self.nonempty(key).filter(|p| p.is_absolute()) {
            return Some(dir);
        }
        let mut dir = self.user_home()?;
        dir.extend(fallback);
        Some(dir)
    }
}

// Rust 1.85 stopped it reading `HOME` on Windows and 1.87 lifted the
// deprecation, so on this crate's 1.86 floor only the warning remains. Before
// 1.90 an empty Unix `HOME` came back as `Some("")`, hence the filter.
#[allow(deprecated)]
fn os_home() -> Option<PathBuf> {
    std::env::home_dir().filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(
        vars: &'static [(&'static str, &'static str)],
    ) -> Dirs<impl Fn(&str) -> Option<OsString>> {
        Dirs::with(move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| OsString::from(v))
        })
    }

    fn p(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn user_home_skips_empty_home_for_userprofile() {
        assert_eq!(dirs(&[("HOME", "/h")]).user_home(), Some(p(&["/h"])));
        assert_eq!(
            dirs(&[("HOME", ""), ("USERPROFILE", "/u")]).user_home(),
            Some(p(&["/u"]))
        );
        assert_eq!(
            dirs(&[("HOME", "/h"), ("USERPROFILE", "/u")]).user_home(),
            Some(p(&["/h"]))
        );
        assert_eq!(dirs(&[("HOME", "")]).user_home(), None);
    }

    #[test]
    fn app_home_prefers_the_variable_and_expands_its_tilde() {
        let d = dirs(&[("HOME", "/h"), ("APP_HOME", "~/x/y")]);
        assert_eq!(d.app_home("APP_HOME", ".app"), Some(p(&["/h", "x", "y"])));
        let d = dirs(&[("HOME", "/h"), ("APP_HOME", "/opt/app")]);
        assert_eq!(d.app_home("APP_HOME", ".app"), Some(p(&["/opt/app"])));
    }

    #[test]
    fn app_home_falls_back_to_the_dot_dir() {
        let d = dirs(&[("HOME", "/h"), ("APP_HOME", "")]);
        assert_eq!(d.app_home("APP_HOME", ".app"), Some(p(&["/h", ".app"])));
        assert_eq!(dirs(&[]).app_home("APP_HOME", ".app"), None);
    }

    #[test]
    fn app_home_keeps_a_relative_variable_for_the_caller_to_judge() {
        let d = dirs(&[("HOME", "/h"), ("APP_HOME", "rel")]);
        let got = d.app_home("APP_HOME", ".app").unwrap();
        assert_eq!(got, p(&["rel"]));
        assert!(!got.is_absolute());
    }

    #[test]
    fn xdg_dirs_use_absolute_values_only() {
        let abs = if cfg!(windows) { "C:\\xdg" } else { "/xdg" };
        let d = Dirs::with(move |key| match key {
            "HOME" => Some(OsString::from("/h")),
            "XDG_CONFIG_HOME" => Some(OsString::from(abs)),
            "XDG_DATA_HOME" => Some(OsString::from("relative")),
            "XDG_CACHE_HOME" => Some(OsString::new()),
            _ => None,
        });
        assert_eq!(d.config_home(), Some(p(&[abs])));
        assert_eq!(d.data_home(), Some(p(&["/h", ".local", "share"])));
        assert_eq!(d.cache_home(), Some(p(&["/h", ".cache"])));
        assert_eq!(d.state_home(), Some(p(&["/h", ".local", "state"])));
    }

    #[test]
    fn xdg_without_any_home_is_none() {
        assert_eq!(dirs(&[]).config_home(), None);
    }

    #[test]
    fn expand_tilde_forms() {
        let home = Path::new("/h");
        assert_eq!(expand_tilde("~", home), p(&["/h"]));
        assert_eq!(expand_tilde("~/a/b", home), p(&["/h", "a", "b"]));
        assert_eq!(
            expand_tilde("~\\.app\\bin", home),
            p(&["/h", ".app", "bin"])
        );
        assert_eq!(expand_tilde("~//a", home), p(&["/h", "a"]));
        assert_eq!(expand_tilde("~user/a", home), p(&["~user/a"]));
        assert_eq!(expand_tilde("a/~/b", home), p(&["a/~/b"]));
        assert_eq!(expand_tilde("/abs", home), p(&["/abs"]));
    }

    #[test]
    fn dirs_expand_tilde_without_a_home_leaves_the_path() {
        assert_eq!(dirs(&[]).expand_tilde("~/a"), p(&["~/a"]));
        assert_eq!(dirs(&[("HOME", "/h")]).expand_tilde("~/a"), p(&["/h", "a"]));
    }

    #[test]
    fn from_env_finds_some_home() {
        // CI runners always have an account home; this proves the OS
        // fallback compiles and answers on each target.
        assert!(user_home().is_some());
    }
}
