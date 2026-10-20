//! Failures while binding an owner-only socket.
//!
//! Messages name the path and the operation. They never include key material;
//! this crate does not handle secrets.

use std::io;
use std::path::PathBuf;

#[cfg(unix)]
use std::path::Path;

/// Why an owner-only Unix socket could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `path` was relative, or it had no parent directory.
    #[error("socket path must be absolute: {0}")]
    InvalidSocketPath(PathBuf),
    /// The parent directory grants group or other access, so another user
    /// could swap the socket.
    #[error("directory is reachable by other users: {0}")]
    InsecureDirectory(PathBuf),
    /// The parent exists and is not a real directory. A symlink is refused:
    /// its target could be a directory this user does not control.
    #[error("not a directory: {0}")]
    NotADirectory(PathBuf),
    /// Something that is not a socket already occupies `path`. It is not
    /// removed.
    #[error("path exists and is not a socket: {0}")]
    NotASocket(PathBuf),
    /// A connect to `path` succeeded or timed out, so a live server still
    /// owns it.
    #[error("socket is already in use: {0}")]
    SocketInUse(PathBuf),
    /// A filesystem call failed.
    #[error("{action} {target}: {source}")]
    Io {
        /// Verb used in the message, such as `bind` or `create`.
        action: &'static str,
        /// Path the call was about.
        target: String,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
}

#[cfg(unix)]
impl Error {
    pub(crate) fn io(action: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            action,
            target: path.display().to_string(),
            source,
        }
    }
}
