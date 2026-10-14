//! Paths a git working tree changed relative to a base ref.
//!
//! The answer is the union of: commits since `git merge-base HEAD <base>`, staged and
//! unstaged edits, untracked files that are not ignored, deleted files and both sides of a
//! rename. It shells out to the `git` CLI (no shell, `-z` output) because every CI and
//! developer machine already has it, and four read-only commands do not justify a heavy
//! library such as `gix`.
//!
//! Callers that cannot trust a partial answer must not treat an [`Error`] as "nothing
//! changed"; failing loudly is the safe choice, and running everything is the other one.

#![deny(missing_docs)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Why the changed paths could not be computed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `git` executable could not be found on `PATH`.
    #[error("git executable not found")]
    GitMissing(#[source] io::Error),
    /// `git` was found but could not be started.
    #[error("failed to run git")]
    Spawn(#[source] io::Error),
    /// The given directory is not inside a git work tree.
    #[error("not a git repository: {path}: {stderr}")]
    NotARepository {
        /// Directory that was inspected.
        path: PathBuf,
        /// Trimmed standard error of git.
        stderr: String,
    },
    /// The base ref does not resolve to a commit.
    #[error("unknown base ref: {0}")]
    BaseUnknown(String),
    /// `HEAD` and the base ref share no history.
    #[error("no merge base between HEAD and {0}")]
    NoMergeBase(String),
    /// `HEAD` and the base ref share no history in a shallow clone, so the merge base is
    /// probably just cut off.
    #[error(
        "no merge base between HEAD and {0}: this is a shallow clone, so the history may be \
         cut off; fetch more (`git fetch --unshallow`, `git fetch --deepen=<n>`, or \
         `fetch-depth: 0` in actions/checkout)"
    )]
    ShallowNoMergeBase(String),
    /// A git command exited unsuccessfully for another reason.
    #[error("`git {args}` failed ({status}): {stderr}")]
    GitFailed {
        /// Arguments of the failed command.
        args: String,
        /// Exit status as printed by the platform.
        status: String,
        /// Trimmed standard error of git.
        stderr: String,
    },
    /// Git printed bytes that are not valid UTF-8 (a path or a sha).
    #[error("git output is not valid UTF-8")]
    NonUtf8(#[source] std::string::FromUtf8Error),
}

/// The result of [`changed_paths`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPaths {
    /// Full sha of `git merge-base HEAD <base>`.
    pub merge_base: String,
    /// Changed paths relative to the repository top level, not to the `repo` argument.
    pub paths: BTreeSet<PathBuf>,
}

/// Paths changed in the work tree of `repo` relative to `base`.
///
/// `repo` may be any directory inside the work tree. Returned paths are relative to the
/// repository top level, so they stay comparable no matter where the caller stands. A
/// rename contributes its old and its new path; ignored files are excluded.
///
/// # Errors
///
/// See [`Error`]: git missing, `repo` not a work tree, unknown `base`, no common history,
/// or output that is not UTF-8.
pub fn changed_paths(repo: &Path, base: &str) -> Result<ChangedPaths, Error> {
    let top = PathBuf::from(
        run(repo, &["rev-parse", "--show-toplevel"], |out, _| {
            Error::NotARepository {
                path: repo.to_path_buf(),
                stderr: stderr_text(out),
            }
        })?
        .trim_end_matches('\n'),
    );

    // A leading dash would be parsed as an option by the commands below.
    let unknown = || Error::BaseUnknown(base.to_owned());
    if base.starts_with('-') {
        return Err(unknown());
    }
    run(
        &top,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
        |_, _| unknown(),
    )?;

    let merge_base = run(&top, &["merge-base", "HEAD", base], |out, args| {
        if out.status.code() == Some(1) {
            if is_shallow(&top) {
                Error::ShallowNoMergeBase(base.to_owned())
            } else {
                Error::NoMergeBase(base.to_owned())
            }
        } else {
            failed(out, args)
        }
    })?
    .trim()
    .to_owned();

    // `--no-renames` reports a rename as delete + add, which yields both sides for free.
    // The work-tree diff covers commits, staged and unstaged edits; the cached diff catches
    // a file staged and then removed from the work tree, which the first one cannot see.
    let mut paths = BTreeSet::new();
    for cached in [false, true] {
        let mut args = vec!["diff", "--name-only", "-z", "--no-renames"];
        if cached {
            args.push("--cached");
        }
        args.push(&merge_base);
        paths.extend(nul_paths(&run(&top, &args, failed)?));
    }
    let untracked = run(
        &top,
        &["ls-files", "-z", "--others", "--exclude-standard"],
        failed,
    )?;
    paths.extend(nul_paths(&untracked));

    Ok(ChangedPaths { merge_base, paths })
}

/// Whether the repository is a shallow clone; any doubt answers `false`, which only picks the
/// plainer error message.
fn is_shallow(top: &Path) -> bool {
    run(top, &["rev-parse", "--is-shallow-repository"], failed)
        .is_ok_and(|out| out.trim() == "true")
}

fn nul_paths(output: &str) -> impl Iterator<Item = PathBuf> + '_ {
    output
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

fn failed(out: &Output, args: &[&str]) -> Error {
    Error::GitFailed {
        args: args.join(" "),
        status: out.status.to_string(),
        stderr: stderr_text(out),
    }
}

fn stderr_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).trim().to_owned()
}

/// Runs `git -C dir args...`; `on_failure` maps a non-zero exit to the caller's error.
fn run(
    dir: &Path,
    args: &[&str],
    on_failure: impl FnOnce(&Output, &[&str]) -> Error,
) -> Result<String, Error> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args.iter().map(OsStr::new))
        // The index is only read; do not make a concurrent `git add` fail on our lock.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Error::GitMissing(e)
            } else {
                Error::Spawn(e)
            }
        })?;
    if !out.status.success() {
        return Err(on_failure(&out, args));
    }
    String::from_utf8(out.stdout).map_err(Error::NonUtf8)
}
