// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later

//! A `MakeWriter` wrapper that masks secrets in every line before it reaches the sink.

use std::io::{self, Write};

use tracing_subscriber::fmt::MakeWriter;

use crate::redact::Redactor;

/// Wraps a sink so everything the formatter writes is scrubbed first. It sits between the
/// formatter and the sink, so the scrubbing also happens before a non-blocking file worker
/// thread ever sees the bytes.
#[derive(Debug, Clone)]
pub struct Scrubbed<M> {
    inner: M,
    redactor: Redactor,
}

impl<M> Scrubbed<M> {
    /// `inner` behind `redactor`.
    pub fn new(inner: M, redactor: Redactor) -> Self {
        Self { inner, redactor }
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for Scrubbed<M> {
    type Writer = ScrubWriter<'a, M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        ScrubWriter {
            inner: self.inner.make_writer(),
            redactor: &self.redactor,
            pending: Vec::new(),
        }
    }
}

/// Buffers one event and scrubs it line by line. Buffering matters: the formatter may write an
/// event in several pieces, and a secret split across two writes would escape a per-write scrub.
#[derive(Debug)]
pub struct ScrubWriter<'a, W: Write> {
    inner: W,
    redactor: &'a Redactor,
    pending: Vec<u8>,
}

impl<W: Write> ScrubWriter<'_, W> {
    /// Scrubs and writes the first `upto` buffered bytes.
    fn emit(&mut self, upto: usize) -> io::Result<()> {
        let ready: Vec<u8> = self.pending.drain(..upto).collect();
        for chunk in ready.split_inclusive(|byte| *byte == b'\n') {
            let (body, newline) = match chunk.strip_suffix(b"\n") {
                Some(body) => (body, true),
                None => (chunk, false),
            };
            let mut line = self.redactor.scrub_line(&String::from_utf8_lossy(body));
            if newline {
                line.push('\n');
            }
            self.inner.write_all(line.as_bytes())?;
        }
        Ok(())
    }
}

impl<W: Write> Write for ScrubWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Only whole lines: scrubbing half a JSON record would miss its structure.
        if let Some(end) = self.pending.iter().rposition(|byte| *byte == b'\n') {
            self.emit(end + 1)?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Drop for ScrubWriter<'_, W> {
    fn drop(&mut self) {
        // A failed log write has nowhere to be reported; losing a line beats panicking in drop.
        let _ = self.emit(self.pending.len());
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::Scrubbed;
    use crate::redact::Redactor;

    /// A sink that records what reaches it.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl MakeWriter<'_> for Sink {
        type Writer = Sink;

        fn make_writer(&self) -> Sink {
            self.clone()
        }
    }

    fn output(sink: &Sink) -> String {
        String::from_utf8(sink.0.lock().unwrap().clone()).unwrap()
    }

    fn scrubbed(sink: &Sink) -> Scrubbed<Sink> {
        Scrubbed::new(sink.clone(), Redactor::new::<&str>(&[]).unwrap())
    }

    #[test]
    fn secret_split_across_writes_is_still_masked() {
        let sink = Sink::default();
        let make = scrubbed(&sink);
        let mut writer = make.make_writer();
        writer.write_all(b"auth Bearer abcdef").unwrap();
        writer.write_all(b"ghijkl012345\nnext line\n").unwrap();
        drop(writer);
        let out = output(&sink);
        assert!(!out.contains("abcdefghijkl"), "{out}");
        assert!(out.contains("next line\n"), "{out}");
    }

    #[test]
    fn flush_emits_only_whole_lines() {
        let sink = Sink::default();
        let make = scrubbed(&sink);
        let mut writer = make.make_writer();
        writer.write_all(b"one\npart").unwrap();
        writer.flush().unwrap();
        assert_eq!(output(&sink), "one\n");
        drop(writer);
        assert_eq!(output(&sink), "one\npart");
    }

    #[test]
    fn json_lines_keep_their_shape() {
        let sink = Sink::default();
        let make = scrubbed(&sink);
        let mut writer = make.make_writer();
        writer
            .write_all(b"{\"fields\":{\"api_key\":\"x\",\"n\":1}}\n")
            .unwrap();
        drop(writer);
        assert_eq!(
            output(&sink),
            "{\"fields\":{\"api_key\":\"[REDACTED]\",\"n\":1}}\n"
        );
    }
}
