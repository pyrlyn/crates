//! The bookkeeping that keeps a call's deadline honest while a person answers
//! a server's question: how many elicitations of one server are open, a way
//! to abort them, and a timeout that does not tick while any is open.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// How many elicitations of one server wait for a person right now. A
/// call's deadline stops while any does (answering must not trip the call
/// timeout), and a cancelled call aborts them.
#[derive(Default)]
pub(crate) struct Asking {
    open: AtomicUsize,
    changed: Notify,
    abort: Mutex<CancellationToken>,
}

impl Asking {
    fn busy(&self) -> bool {
        self.open.load(Ordering::SeqCst) > 0
    }

    /// Counts one open elicitation until the guard drops, however the
    /// handler's future ends.
    pub(crate) fn enter(self: &Arc<Self>) -> Open {
        self.open.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_waiters();
        Open(self.clone())
    }

    /// The token the open elicitations watch.
    pub(crate) fn abort_token(&self) -> CancellationToken {
        self.abort
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Cancels every open elicitation; later ones get a fresh token.
    pub(crate) fn abort_all(&self) {
        let mut token = self.abort.lock().unwrap_or_else(PoisonError::into_inner);
        token.cancel();
        *token = CancellationToken::new();
    }
}

pub(crate) struct Open(Arc<Asking>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

/// Runs `fut` with a `timeout` that does not count while a question is
/// open; each time the last question closes the full timeout starts again.
/// `None` is a timeout.
pub(crate) async fn within<F: Future>(
    asking: &Asking,
    timeout: Duration,
    fut: F,
) -> Option<F::Output> {
    tokio::pin!(fut);
    loop {
        let changed = asking.changed.notified();
        tokio::pin!(changed);
        // Registered before `busy` is read, so a change in between still
        // wakes this loop.
        changed.as_mut().enable();
        if asking.busy() {
            tokio::select! {
                out = &mut fut => return Some(out),
                () = &mut changed => continue,
            }
        }
        tokio::select! {
            out = &mut fut => return Some(out),
            () = tokio::time::sleep(timeout) => return None,
            () = &mut changed => {}
        }
    }
}
