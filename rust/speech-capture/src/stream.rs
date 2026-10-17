// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! The thread every `cpal` stream here lives on, shared by input and output.
//! A `cpal` stream is not `Send` on every platform, so it is built, played and
//! dropped on one thread; the guard that ends that thread is `Send`.

use std::sync::mpsc;
use std::thread::JoinHandle;

use crate::Error;

/// Dropping it stops the stream and waits for its thread, so no callback runs
/// after the drop returns.
#[derive(Debug)]
pub(crate) struct StreamThread {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl StreamThread {
    /// Runs `open` on a new thread and returns once the stream it built is
    /// playing, or with the error that stopped it.
    pub(crate) fn spawn(
        name: &str,
        open: impl FnOnce() -> Result<cpal::Stream, Error> + Send + 'static,
    ) -> Result<Self, Error> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || match open() {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    // Returns once the sender is dropped: stop or cancel.
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| Error::Stream(e.to_string()))?;
        // Built before the wait so a failed start still joins the thread.
        let guard = Self {
            stop: Some(stop_tx),
            thread: Some(thread),
        };
        ready_rx
            .recv()
            .map_err(|_| Error::Stream("the stream thread ended".into()))??;
        Ok(guard)
    }
}

impl Drop for StreamThread {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
