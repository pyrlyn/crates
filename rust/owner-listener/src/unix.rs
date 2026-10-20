//! Owner-only Unix domain socket.
//!
//! The parent directory is created mode `0700`, or accepted only when it
//! already is a real directory that other users cannot enter. The socket
//! file is mode `0600`. A socket left behind by a crash is removed; one that
//! still accepts a connection is left alone.

use std::fs::{self, DirBuilder, Permissions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::unix::UCred;
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::Stream;
use tonic::transport::server::Connected;

use crate::error::Error;

/// How long a connect may take before the socket is treated as live.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Effective user id of this process.
pub fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Credentials tonic stores on each accepted connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnixConnectInfo {
    /// Effective uid of the peer.
    pub uid: u32,
    /// Effective gid of the peer.
    pub gid: u32,
    /// Peer pid, when the kernel reported one.
    pub pid: Option<i32>,
}

impl UnixConnectInfo {
    /// Whether this peer is `owner_uid`.
    ///
    /// The pid is not an identity: it is exposed so a caller can name the
    /// process in an approval prompt.
    pub fn is_owner(&self, owner_uid: u32) -> bool {
        self.uid == owner_uid
    }
}

/// One accepted connection and the credentials taken at accept time.
#[derive(Debug)]
pub struct UnixIo {
    stream: UnixStream,
    info: UnixConnectInfo,
}

impl UnixIo {
    /// Credentials of the process that connected.
    pub fn info(&self) -> UnixConnectInfo {
        self.info.clone()
    }

    /// The underlying socket.
    pub fn into_inner(self) -> UnixStream {
        self.stream
    }
}

impl Connected for UnixIo {
    type ConnectInfo = UnixConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.info()
    }
}

impl AsyncRead for UnixIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Deletes the socket file when the listener stops.
#[derive(Debug)]
struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.0)
            && e.kind() != ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.0.display(),
                error = %e,
                "could not remove the socket file"
            );
        }
    }
}

/// Accepted connections. Dropping this removes the socket file.
#[derive(Debug)]
pub struct UnixIncoming {
    listener: UnixListener,
    guard: SocketGuard,
}

impl UnixIncoming {
    /// Path of the socket file.
    pub fn path(&self) -> &Path {
        &self.guard.0
    }
}

impl Stream for UnixIncoming {
    type Item = io::Result<UnixIo>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut().listener.poll_accept(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Ready(Ok((stream, _addr))) => Poll::Ready(Some(accepted(stream))),
        }
    }
}

fn accepted(stream: UnixStream) -> io::Result<UnixIo> {
    let cred = stream.peer_cred()?;
    Ok(UnixIo {
        info: info_from(&cred),
        stream,
    })
}

fn info_from(cred: &UCred) -> UnixConnectInfo {
    UnixConnectInfo {
        uid: cred.uid(),
        gid: cred.gid(),
        pid: cred.pid(),
    }
}

/// Binds `path` as an owner-only socket.
///
/// `path` must be absolute. Between `bind` and the `0600` chmod, the `0700`
/// parent is what keeps other users out.
pub async fn bind(path: &Path) -> Result<UnixIncoming, Error> {
    let parent = path
        .parent()
        .filter(|parent| path.is_absolute() && !parent.as_os_str().is_empty())
        .ok_or_else(|| Error::InvalidSocketPath(path.to_owned()))?;
    owner_only_dir(parent)?;
    clear_stale(path).await?;
    let listener = UnixListener::bind(path).map_err(|source| Error::io("bind", path, source))?;
    fs::set_permissions(path, Permissions::from_mode(0o600))
        .map_err(|source| Error::io("restrict", path, source))?;
    Ok(UnixIncoming {
        listener,
        guard: SocketGuard(path.to_owned()),
    })
}

fn owner_only_dir(dir: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(dir) {
        Ok(meta) => ensure_owner_only(dir, &meta),
        Err(source) if source.kind() == ErrorKind::NotFound => {
            match DirBuilder::new().recursive(true).mode(0o700).create(dir) {
                Ok(()) => {}
                Err(source) if source.kind() == ErrorKind::AlreadyExists => {}
                Err(source) => return Err(Error::io("create", dir, source)),
            }
            let meta =
                fs::symlink_metadata(dir).map_err(|source| Error::io("inspect", dir, source))?;
            ensure_owner_only(dir, &meta)
        }
        Err(source) => Err(Error::io("inspect", dir, source)),
    }
}

fn ensure_owner_only(dir: &Path, meta: &fs::Metadata) -> Result<(), Error> {
    let kind = meta.file_type();
    if kind.is_symlink() || !kind.is_dir() {
        return Err(Error::NotADirectory(dir.to_owned()));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(Error::InsecureDirectory(dir.to_owned()));
    }
    Ok(())
}

/// Removes a socket whose listener is gone. A non-socket is not ours to delete.
async fn clear_stale(path: &Path) -> Result<(), Error> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(Error::io("inspect", path, source)),
    };
    let kind = meta.file_type();
    if kind.is_symlink() || !kind.is_socket() {
        return Err(Error::NotASocket(path.to_owned()));
    }
    match tokio::time::timeout(PROBE_TIMEOUT, UnixStream::connect(path)).await {
        Ok(Err(source)) if source.kind() == ErrorKind::ConnectionRefused => {
            tracing::info!(path = %path.display(), "removing a stale socket");
            fs::remove_file(path).map_err(|source| Error::io("remove stale", path, source))
        }
        Ok(Err(source)) => Err(Error::io("probe", path, source)),
        Ok(Ok(_)) | Err(_) => Err(Error::SocketInUse(path.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener as StdListener;

    use tokio_stream::StreamExt;

    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn bind_restricts_modes_and_reports_the_peer() {
        let dir = tempdir();
        let path = dir.path().join("run").join("agent.sock");
        let mut incoming = bind(&path).await.unwrap();
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(mode(&path), 0o600);

        let path_for_client = path.clone();
        let client =
            tokio::spawn(async move { UnixStream::connect(path_for_client).await.unwrap() });
        let io = tokio::time::timeout(Duration::from_secs(2), incoming.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let info = io.info();
        assert_eq!(info.uid, current_uid());
        assert_eq!(info.pid, Some(i32::try_from(std::process::id()).unwrap()));
        assert!(info.is_owner(current_uid()));
        assert!(!info.is_owner(current_uid().wrapping_add(1)));
        assert_eq!(io.connect_info(), info);
        client.await.unwrap();
        drop(incoming);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_existing_private_directory_is_reused() {
        let dir = tempdir();
        let parent = dir.path().join("run");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(0o700)).unwrap();
        let path = parent.join("agent.sock");
        let incoming = bind(&path).await.unwrap();
        assert_eq!(mode(&parent), 0o700);
        drop(incoming);
    }

    #[tokio::test]
    async fn a_directory_other_users_can_enter_is_refused() {
        let dir = tempdir();
        let parent = dir.path().join("open");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(0o755)).unwrap();
        let path = parent.join("agent.sock");
        let err = bind(&path).await.unwrap_err();
        assert!(matches!(err, Error::InsecureDirectory(_)), "{err}");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_symlink_parent_is_refused() {
        let dir = tempdir();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        symlink(&real, &link).unwrap();
        let err = bind(&link.join("agent.sock")).await.unwrap_err();
        assert!(matches!(err, Error::NotADirectory(_)), "{err}");
    }

    #[tokio::test]
    async fn a_regular_file_is_not_deleted() {
        let dir = tempdir();
        let parent = dir.path().join("run");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(0o700)).unwrap();
        let path = parent.join("agent.sock");
        fs::write(&path, b"not a socket").unwrap();
        let err = bind(&path).await.unwrap_err();
        assert!(matches!(err, Error::NotASocket(_)), "{err}");
        assert_eq!(fs::read(&path).unwrap(), b"not a socket");
    }

    #[tokio::test]
    async fn a_stale_socket_is_replaced() {
        let dir = tempdir();
        let parent = dir.path().join("run");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(0o700)).unwrap();
        let path = parent.join("agent.sock");
        {
            let _no_spawn = crate::SPAWN_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(StdListener::bind(&path).unwrap());
        }
        assert!(path.exists());
        let incoming = bind(&path).await.unwrap();
        assert_eq!(mode(&path), 0o600);
        drop(incoming);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_live_socket_is_left_in_place() {
        let dir = tempdir();
        let parent = dir.path().join("run");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, Permissions::from_mode(0o700)).unwrap();
        let path = parent.join("agent.sock");
        let live = StdListener::bind(&path).unwrap();
        let err = bind(&path).await.unwrap_err();
        assert!(matches!(err, Error::SocketInUse(_)), "{err}");
        assert!(path.exists());
        drop(live);
    }

    #[tokio::test]
    async fn a_relative_path_is_refused() {
        let err = bind(Path::new("agent.sock")).await.unwrap_err();
        assert!(matches!(err, Error::InvalidSocketPath(_)), "{err}");
    }
}
