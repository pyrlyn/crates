//! Windows named-pipe listener.
//!
//! The first instance is created with `first_pipe_instance`, so another
//! process cannot squat the name. Every instance gets a DACL for the current
//! user's SID only, and remote clients are rejected. Each accepted connection
//! carries the client pid, and a client whose user SID is not the owner's is
//! dropped before tonic sees it.
//!
//! Authenticode checks are the caller's decision: this crate reports the
//! server pid and does not pick a trusted signer.

use std::io::{self, ErrorKind};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::transport::server::Connected;

use crate::pipe_name::validate_pipe_name;
use crate::security::{self, UserSid};

/// Client pid of one accepted pipe connection.
///
/// The connection is only produced after the client's user SID matched the
/// account that created the pipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeConnectInfo {
    /// Process id of the client, from `GetNamedPipeClientProcessId`.
    pub client_pid: u32,
}

/// One accepted server instance.
#[derive(Debug)]
pub struct PipeIo {
    io: NamedPipeServer,
    info: PipeConnectInfo,
}

impl PipeIo {
    /// Client pid captured when the instance connected.
    pub fn info(&self) -> PipeConnectInfo {
        self.info.clone()
    }

    /// The underlying pipe instance.
    pub fn into_inner(self) -> NamedPipeServer {
        self.io
    }
}

impl Connected for PipeIo {
    type ConnectInfo = PipeConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        self.info()
    }
}

impl AsyncRead for PipeIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for PipeIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// Accepted connections. Dropping this stops the listener task.
#[derive(Debug)]
pub struct PipeIncoming {
    inner: ReceiverStream<io::Result<PipeIo>>,
    stop: CancellationToken,
}

impl Drop for PipeIncoming {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl Stream for PipeIncoming {
    type Item = io::Result<PipeIo>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().inner).poll_next(cx)
    }
}

/// Binds `name` (for example `\\.\pipe\cofre-agent`) as an owner-only pipe.
pub fn bind(name: &str) -> io::Result<PipeIncoming> {
    validate_pipe_name(name)?;
    let sid = UserSid::current()?;
    let first = security::create_server(name, &sid, true)?;
    Ok(spawn(first, name.to_owned(), sid))
}

fn spawn(first: NamedPipeServer, name: String, sid: UserSid) -> PipeIncoming {
    let stop = CancellationToken::new();
    let (tx, rx) = mpsc::channel(1);
    let cancel = stop.clone();
    tokio::spawn(async move {
        let mut spare = first;
        loop {
            let connected = tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                result = spare.connect() => result,
            };
            let next = match security::create_server(&name, &sid, false) {
                Ok(next) => next,
                Err(error) => {
                    tracing::error!(%error, "named pipe listener stopped");
                    let _ = tx.send(Err(error)).await;
                    break;
                }
            };
            let current = std::mem::replace(&mut spare, next);
            if let Err(error) = connected {
                tracing::warn!(%error, "named pipe connect failed");
                continue;
            }
            let Ok(client_pid) = security::client_process_id(&current) else {
                tracing::warn!("named pipe client pid unavailable; refusing the client");
                continue;
            };
            match security::process_user_sid(client_pid) {
                Ok(peer) if peer == sid => {}
                Ok(_) => {
                    tracing::warn!(
                        client_pid,
                        "refused a named pipe client that is not the owner"
                    );
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        client_pid,
                        %error,
                        "refused a named pipe client whose user could not be read"
                    );
                    continue;
                }
            }
            let io = PipeIo {
                io: current,
                info: PipeConnectInfo { client_pid },
            };
            if tx.send(Ok(io)).await.is_err() {
                break;
            }
        }
    });
    PipeIncoming {
        inner: ReceiverStream::new(rx),
        stop,
    }
}

/// A client connection plus the server process id.
///
/// Compare [`PipeClient::server_pid`] with the agent process you started.
/// A signature check, if you want one, uses that pid and a signer you trust;
/// this crate does not choose a signer.
#[derive(Debug)]
pub struct PipeClient {
    io: NamedPipeClient,
    server_pid: u32,
}

impl PipeClient {
    /// Process id of the server end, from `GetNamedPipeServerProcessId`.
    pub fn server_pid(&self) -> u32 {
        self.server_pid
    }

    /// The underlying client pipe.
    pub fn into_inner(self) -> NamedPipeClient {
        self.io
    }
}

impl AsyncRead for PipeClient {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for PipeClient {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// Connects to `name` and reads the server pid.
///
/// This calls Win32 `CreateFile` on the current thread, the same way
/// `ClientOptions::open` does. A pipe that is busy returns that operating
/// system error; the caller decides whether to retry. The pid is required:
/// a pipe whose server cannot be identified is not used.
pub fn connect(name: &str) -> io::Result<PipeClient> {
    validate_pipe_name(name)?;
    let io = ClientOptions::new().open(name)?;
    let server_pid = security::server_process_id(&io)?;
    if server_pid == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "pipe server pid was 0",
        ));
    }
    Ok(PipeClient { io, server_pid })
}
