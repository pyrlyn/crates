//! Git sources: fetch one commit into the cache and pin it by SHA.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache::sha256_hex;
use plugwright_core::error::{PluginError, Result, io_err};

const COMPLETE_MARKER: &str = ".plugwright-complete";

/// Which commit to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRef {
    /// Full 40-hex commit SHA (or any ref name).
    Rev(String),
    Branch(String),
    Tag(String),
    /// The remote's `HEAD`.
    DefaultBranch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitSource {
    pub url: String,
    pub reference: GitRef,
    /// Crate directory inside the repository (monorepos).
    pub subdir: Option<PathBuf>,
}

impl GitSource {
    /// Stable description, e.g. `git+https://host/repo?branch=main#<sha>`.
    pub fn label(&self, sha: &str) -> String {
        let r = match &self.reference {
            GitRef::Rev(r) => format!("?rev={r}"),
            GitRef::Branch(b) => format!("?branch={b}"),
            GitRef::Tag(t) => format!("?tag={t}"),
            GitRef::DefaultBranch => String::new(),
        };
        format!("git+{}{r}#{sha}", self.url)
    }
}

pub(crate) fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reject URLs that could smuggle options or exotic transports into git.
pub fn validate_url(url: &str, allow_local: bool) -> Result<()> {
    let bad = |why: &str| Err(PluginError::Git(format!("refusing git URL `{url}`: {why}")));
    if url.is_empty() || url.starts_with('-') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("empty, starts with `-`, or contains whitespace");
    }
    let scp_like = url.starts_with("git@") && url.contains(':') && !url.contains("://");
    if url.starts_with("https://") || url.starts_with("ssh://") || scp_like {
        return Ok(());
    }
    if url.starts_with("file://") {
        return if allow_local { Ok(()) } else { bad("local repositories need `allow_local_git = true`") };
    }
    bad("only https://, ssh:// and git@host:path are allowed")
}

fn validate_ref(r: &str) -> Result<()> {
    let ok = !r.is_empty()
        && !r.starts_with('-')
        && !r.contains("..")
        && r.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
    if ok { Ok(()) } else { Err(PluginError::Git(format!("invalid git ref `{r}`"))) }
}

fn validate_subdir(p: &Path) -> Result<()> {
    if p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
        Ok(())
    } else {
        Err(PluginError::Git(format!("subdir `{}` must be a relative path inside the repository", p.display())))
    }
}

fn git(dir: &Path, allow_local: bool) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir)
        .args(["-c", "protocol.ext.allow=never"])
        .args(["-c", if allow_local { "protocol.file.allow=always" } else { "protocol.file.allow=never" }])
        .args(["-c", "core.hooksPath=/dev/null", "-c", "advice.detachedHead=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<String> {
    let out = cmd.output().map_err(|e| PluginError::Git(format!("cannot run git ({what}): {e}")))?;
    if !out.status.success() {
        return Err(PluginError::Git(format!("git {what} failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

static TMP: AtomicU64 = AtomicU64::new(0);

/// Fetch `src` into `git_cache` and return `(crate_dir, commit_sha)`.
///
/// With `pinned` (a SHA from the lockfile) an existing checkout is reused
/// without touching the network.
pub fn checkout(
    git_cache: &Path,
    src: &GitSource,
    pinned: Option<&str>,
    allow_local: bool,
    offline: bool,
) -> Result<(PathBuf, String)> {
    validate_url(&src.url, allow_local)?;
    if let Some(s) = &src.subdir {
        validate_subdir(s)?;
    }
    let url_key = &sha256_hex(src.url.as_bytes())[..16];
    let crate_dir = |root: PathBuf| match &src.subdir {
        Some(s) => root.join(s),
        None => root,
    };

    let want_sha = pinned.map(str::to_owned).or_else(|| match &src.reference {
        GitRef::Rev(r) if is_full_sha(r) => Some(r.to_lowercase()),
        _ => None,
    });
    if let Some(sha) = &want_sha {
        let dir = git_cache.join(format!("{url_key}-{sha}"));
        if dir.join(COMPLETE_MARKER).exists() {
            tracing::debug!(url = %src.url, sha, "git checkout cache hit");
            return Ok((crate_dir(dir), sha.clone()));
        }
    }
    if offline {
        return Err(PluginError::Git(format!("{} is not in the cache and offline mode is on", src.url)));
    }

    let fetch_ref = match (&want_sha, &src.reference) {
        (Some(sha), _) => sha.clone(),
        (None, GitRef::Rev(r)) => r.clone(),
        (None, GitRef::Branch(b)) => format!("refs/heads/{b}"),
        (None, GitRef::Tag(t)) => format!("refs/tags/{t}"),
        (None, GitRef::DefaultBranch) => "HEAD".into(),
    };
    validate_ref(&fetch_ref)?;

    std::fs::create_dir_all(git_cache).map_err(io_err(git_cache))?;
    let tmp = git_cache.join(format!(".tmp-{}-{}", std::process::id(), TMP.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&tmp).map_err(io_err(&tmp))?;
    let result = (|| {
        let mut c = git(&tmp, allow_local);
        c.args(["init", "-q"]);
        run(c, "init")?;
        let mut c = git(&tmp, allow_local);
        c.args(["fetch", "-q", "--depth", "1", "--no-recurse-submodules", &src.url, &fetch_ref]);
        tracing::info!(url = %src.url, reference = %fetch_ref, "fetching git source");
        run(c, "fetch")?;
        let mut c = git(&tmp, allow_local);
        c.args(["checkout", "-q", "--detach", "FETCH_HEAD"]);
        run(c, "checkout")?;
        let mut c = git(&tmp, allow_local);
        c.args(["rev-parse", "HEAD"]);
        run(c, "rev-parse")
    })();
    let sha = match result {
        Ok(sha) => sha.to_lowercase(),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    if let Some(want) = &want_sha
        && want != &sha
    {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(PluginError::Git(format!("fetched commit {sha} but {want} was pinned")));
    }
    let dir = git_cache.join(format!("{url_key}-{sha}"));
    if dir.join(COMPLETE_MARKER).exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir).map_err(io_err(&dir))?;
        std::fs::write(dir.join(COMPLETE_MARKER), &sha).map_err(io_err(&dir))?;
    }
    tracing::info!(url = %src.url, sha, "git source ready");
    Ok((crate_dir(dir), sha))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_policy() {
        assert!(validate_url("https://github.com/a/b", false).is_ok());
        assert!(validate_url("ssh://git@host/a", false).is_ok());
        assert!(validate_url("git@github.com:a/b.git", false).is_ok());
        assert!(validate_url("file:///tmp/x", false).is_err());
        assert!(validate_url("file:///tmp/x", true).is_ok());
        assert!(validate_url("ext::sh -c touch% /tmp/pwned", true).is_err());
        assert!(validate_url("--upload-pack=evil", true).is_err());
        assert!(validate_url("http://insecure", true).is_err());
    }

    #[test]
    fn ref_and_subdir_policy() {
        assert!(validate_ref("refs/heads/main").is_ok());
        assert!(validate_ref("-x").is_err());
        assert!(validate_ref("a..b").is_err());
        assert!(validate_subdir(Path::new("plugins/x")).is_ok());
        assert!(validate_subdir(Path::new("../escape")).is_err());
        assert!(validate_subdir(Path::new("/abs")).is_err());
    }

    #[test]
    fn url_policy_allows_only_authenticated_remote_schemes() {
        for ok in ["https://h/r.git", "ssh://git@h:22/r", "git@h:r.git", "git@h:org/r"] {
            assert!(validate_url(ok, false).is_ok(), "{ok}");
        }
        let refused = [
            "",
            "http://h/r",
            "git://h/r",
            "ftp://h/r",
            "ext::sh -c id",
            "ext::nc%20h%2022",
            "fd::3",
            "https::h/r",
            "/srv/repo",
            "./repo",
            "../repo",
            "repo",
            "C:\\repo",
            "git@h",
            "git@h://r",
            "-uhttps://h/r",
            "https://h/r -c x",
            "https://h/\nr",
            "https://h/\tr",
        ];
        for bad in refused {
            for allow_local in [false, true] {
                match validate_url(bad, allow_local) {
                    Err(PluginError::Git(m)) => assert!(m.contains("refusing git URL"), "{m}"),
                    other => panic!("{bad:?} (allow_local={allow_local}) must be refused, got {other:?}"),
                }
            }
        }
    }

    #[test]
    fn local_repositories_need_opt_in_and_a_file_url() {
        let err = validate_url("file:///srv/repo", false).unwrap_err().to_string();
        assert!(err.contains("allow_local_git = true"), "{err}");
        validate_url("file:///srv/repo", true).unwrap();
        // Bare paths stay refused even with the opt-in: only `file://` is local.
        assert!(validate_url("/srv/repo", true).unwrap_err().to_string().contains("only https://"));
    }

    #[test]
    fn refs_are_limited_to_safe_names() {
        for ok in ["main", "refs/tags/v1.2.3", "HEAD", "feature/a_b-c", &"a".repeat(40)] {
            assert!(validate_ref(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-b", "--upload-pack=x", "a..b", "a b", "a~1", "a^", "a:b", "@{-1}", "a\\b", "a*"] {
            assert!(matches!(validate_ref(bad), Err(PluginError::Git(m)) if m.contains("invalid git ref")), "{bad}");
        }
    }

    #[test]
    fn subdirs_must_stay_inside_the_repository() {
        for ok in ["", ".", "./a", "a/./b"] {
            assert!(validate_subdir(Path::new(ok)).is_ok(), "{ok}");
        }
        for bad in ["..", "a/../b", "a/..", "/a"] {
            assert!(validate_subdir(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn full_sha_detection_and_source_labels() {
        assert!(is_full_sha(&"aB".repeat(20)));
        assert!(!is_full_sha(&"a".repeat(39)) && !is_full_sha(&"a".repeat(41)) && !is_full_sha(&"g".repeat(40)));
        let src = |reference| GitSource { url: "https://h/r".into(), reference, subdir: None };
        assert_eq!(src(GitRef::DefaultBranch).label("s"), "git+https://h/r#s");
        assert_eq!(src(GitRef::Branch("main".into())).label("s"), "git+https://h/r?branch=main#s");
        assert_eq!(src(GitRef::Tag("v1".into())).label("s"), "git+https://h/r?tag=v1#s");
        assert_eq!(src(GitRef::Rev("abc".into())).label(""), "git+https://h/r?rev=abc#");
    }

    /// A finished checkout as `checkout` leaves it, without running git.
    fn cached_checkout(cache: &Path, url: &str, sha: &str) -> PathBuf {
        let dir = cache.join(format!("{}-{sha}", &sha256_hex(url.as_bytes())[..16]));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join(COMPLETE_MARKER), sha).unwrap();
        dir
    }

    #[test]
    fn pinned_or_full_sha_checkouts_are_served_from_cache_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let url = "https://example.invalid/r.git";
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let dir = cached_checkout(tmp.path(), url, sha);
        let src = |reference| GitSource { url: url.into(), reference, subdir: Some("sub".into()) };

        let got = checkout(tmp.path(), &src(GitRef::Branch("main".into())), Some(sha), false, true).unwrap();
        assert_eq!(got, (dir.join("sub"), sha.to_owned()));
        // A full SHA in `rev` needs no lockfile pin; it is matched case-insensitively.
        let got = checkout(tmp.path(), &src(GitRef::Rev(sha.to_uppercase())), None, false, true).unwrap();
        assert_eq!(got, (dir.join("sub"), sha.to_owned()));
    }

    #[test]
    fn offline_mode_never_fetches() {
        let tmp = tempfile::tempdir().unwrap();
        let url = "https://example.invalid/r.git";
        let sha = "0123456789abcdef0123456789abcdef01234567";
        // An interrupted checkout (no completion marker) is not a cache hit.
        let dir = cached_checkout(tmp.path(), url, sha);
        std::fs::remove_file(dir.join(COMPLETE_MARKER)).unwrap();
        let src = GitSource { url: url.into(), reference: GitRef::Branch("main".into()), subdir: None };
        for pinned in [None, Some(sha)] {
            let err = checkout(tmp.path(), &src, pinned, false, true).unwrap_err().to_string();
            assert!(err.contains("offline mode is on"), "{err}");
        }
    }

    #[test]
    fn checkout_validates_before_touching_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("git");
        let src = |url: &str, reference, subdir: Option<&str>| GitSource {
            url: url.into(),
            reference,
            subdir: subdir.map(PathBuf::from),
        };
        let cases = [
            (src("ext::sh -c id", GitRef::DefaultBranch, None), "refusing git URL"),
            (src("file:///srv/r", GitRef::DefaultBranch, None), "allow_local_git"),
            (src("https://h/r", GitRef::DefaultBranch, Some("../up")), "inside the repository"),
            (src("https://h/r", GitRef::Rev("--upload-pack=x".into()), None), "invalid git ref"),
            (src("https://h/r", GitRef::Branch("a..b".into()), None), "invalid git ref"),
            (src("https://h/r", GitRef::Tag("v 1".into()), None), "invalid git ref"),
        ];
        for (s, needle) in cases {
            let err = checkout(&cache, &s, None, false, false).unwrap_err().to_string();
            assert!(err.contains(needle), "{s:?}: {err}");
        }
        assert!(!cache.exists(), "nothing is created for a refused source");
    }

    #[test]
    fn fetches_local_repo_and_pins_sha() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("sub/file.txt"), "hi").unwrap();
        let sh = |args: &[&str]| {
            let ok = Command::new("git")
                .current_dir(&repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        sh(&["init", "-q", "-b", "main"]);
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "init"]);

        let src = GitSource {
            url: format!("file://{}", repo.display()),
            reference: GitRef::Branch("main".into()),
            subdir: Some("sub".into()),
        };
        let cache = tmp.path().join("cache");
        assert!(checkout(&cache, &src, None, false, false).is_err(), "file:// needs opt-in");
        let (dir, sha) = checkout(&cache, &src, None, true, false).unwrap();
        assert!(is_full_sha(&sha));
        assert_eq!(std::fs::read_to_string(dir.join("file.txt")).unwrap(), "hi");
        // Pinned + offline: served from the cache.
        let (dir2, sha2) = checkout(&cache, &src, Some(&sha), true, true).unwrap();
        assert_eq!((dir, sha), (dir2, sha2));
    }
}
