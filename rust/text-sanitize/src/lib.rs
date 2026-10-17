// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or the royalty-free licence in LICENSE-ROYALTY-FREE.md.

//! Make text somebody else wrote safe to print.
//!
//! A model, a tool, an MCP server, a changelog or a saved request body can carry bytes that act
//! on the terminal instead of being shown by it: an escape sequence moves the cursor, sets the
//! title or writes the clipboard, a bidi override reorders what the reader sees, a zero-width
//! run hides text. [`clean`] removes them; one [`Options`] value says how much, so the callers
//! that need different output (cox, ketch, rtok) share one scanner instead of three.
//!
//! Presets, one per caller:
//!
//! | Preset | Function | Parses sequences | `\r` | Invisibles | Bidi |
//! | --- | --- | --- | --- | --- | --- |
//! | [`Options::TERMINAL`] | [`sanitize`], [`sanitize_with`] | yes | dropped | runs, joiners kept | removed |
//! | [`Options::PROSE`] | [`sanitize_prose`] | no, control characters only | dropped | all | removed |
//! | [`Options::RECORD`] | [`terminal_noise`] | yes | kept | zero-width, joiners kept | kept |
//!
//! ```
//! use text_sanitize::sanitize;
//! assert_eq!(sanitize("\u{1b}]0;pwned\u{7}ok\u{202e}!"), "ok!");
//! ```
//!
//! `\n` and `\t` always stay. Everything is pure and the output of every preset is itself clean:
//! cleaning twice changes nothing.

use std::borrow::Cow;

/// What to do with escape sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escapes {
    /// Remove a whole sequence: CSI, OSC, DCS, SOS, PM and APC with their payload, two- and
    /// three-character escapes, and the 8-bit C1 forms of the introducers.
    Sequences,
    /// Remove only the control characters (ESC, BEL, C1). The payload of a sequence stays as
    /// the harmless text it has become: `ESC [ 2 J` leaves `[2J`.
    ControlsOnly,
}

/// What to do with a carriage return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnds {
    /// Remove it: a lone `\r` rewrites the line the reader is on.
    Drop,
    /// Leave it, for text a model reads back byte for byte.
    Keep,
    /// `\r\n` and a lone `\r` become `\n`.
    Lf,
}

/// What to do with characters that take no space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invisibles {
    /// Leave them.
    Keep,
    /// Remove each U+200B, U+2060 and U+FEFF; U+200C and U+200D stay, because they join emoji
    /// and shape scripts.
    ZeroWidth,
    /// Remove a run of U+200B..U+200D, U+2060 and U+FEFF, except a lone ZWJ or ZWNJ between two
    /// visible characters.
    ZeroWidthRuns,
    /// Remove every one: soft hyphen, Arabic letter mark, U+200B..U+200F, line and paragraph
    /// separators, U+202A..U+202E, U+2060..U+206F, U+FEFF and the tag block U+E0000..U+E007F.
    /// This includes the bidi controls whatever [`Options::bidi`] says, and the joiners.
    All,
}

/// How [`clean`] treats each class of character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Leave a glyph where something was removed (`␛` for an escape sequence, a control
    /// picture for a C0 byte, `⇄` for a bidi control, `∅` for an invisible), so a verbose user
    /// sees that text was cut.
    pub marks: bool,
    /// Escape sequences.
    pub escapes: Escapes,
    /// Carriage returns.
    pub line_ends: LineEnds,
    /// Zero-width and other invisible characters.
    pub invisibles: Invisibles,
    /// Remove bidi overrides and isolates (U+202A..U+202E, U+2066..U+2069).
    pub bidi: bool,
}

impl Options {
    /// Text bound for a terminal: cox's guard on everything a model, tool or server wrote.
    pub const TERMINAL: Self = Self {
        marks: false,
        escapes: Escapes::Sequences,
        line_ends: LineEnds::Drop,
        invisibles: Invisibles::ZeroWidthRuns,
        bidi: true,
    };
    /// Prose from a client app (ketch's changelogs, status lines and tables): drop every
    /// control and invisible character where the text enters, keep what is left.
    pub const PROSE: Self = Self {
        marks: false,
        escapes: Escapes::ControlsOnly,
        line_ends: LineEnds::Drop,
        invisibles: Invisibles::All,
        bidi: true,
    };
    /// A record the model reads back (rtok's saved bodies and tool results): escapes, control
    /// and zero-width characters go, `\r` and bidi characters stay so edits still match.
    pub const RECORD: Self = Self {
        marks: false,
        escapes: Escapes::Sequences,
        line_ends: LineEnds::Keep,
        invisibles: Invisibles::ZeroWidth,
        bidi: false,
    };
}

/// [`Options::TERMINAL`].
pub fn sanitize(s: &str) -> String {
    sanitize_with(s, false)
}

/// [`Options::TERMINAL`] with `marks` as given.
pub fn sanitize_with(s: &str, marks: bool) -> String {
    clean(
        s,
        &Options {
            marks,
            ..Options::TERMINAL
        },
    )
    .into_owned()
}

/// [`Options::PROSE`].
pub fn sanitize_prose(s: &str) -> String {
    clean(s, &Options::PROSE).into_owned()
}

/// [`Options::RECORD`]; borrows `s` when there was nothing to remove.
pub fn terminal_noise(s: &str) -> Cow<'_, str> {
    clean(s, &Options::RECORD)
}

/// `s` with what `options` names removed; borrows `s` when nothing was.
pub fn clean<'a>(s: &'a str, options: &Options) -> Cow<'a, str> {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(c) = s.get(i..).and_then(|rest| rest.chars().next()) {
        let next = i + c.len_utf8();
        i = next;
        match c {
            '\u{1b}' if options.escapes == Escapes::Sequences => {
                i = skip_escape(bytes, next - 1);
                mark(&mut out, options, '␛');
            }
            // The 8-bit forms: CSI, then DCS, SOS, OSC, PM and APC.
            '\u{9b}' if options.escapes == Escapes::Sequences => {
                i = skip_csi(bytes, next);
                mark(&mut out, options, '␛');
            }
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}'
                if options.escapes == Escapes::Sequences =>
            {
                i = skip_string(bytes, next);
                mark(&mut out, options, '␛');
            }
            '\n' | '\t' => out.push(c),
            '\r' => match options.line_ends {
                LineEnds::Keep => out.push(c),
                // The LF of a CRLF follows and is kept on its own.
                LineEnds::Lf if bytes.get(next) == Some(&b'\n') => {}
                LineEnds::Lf => out.push('\n'),
                LineEnds::Drop => mark(&mut out, options, '␍'),
            },
            // C0, DEL and C1 (`is_control`), including a lone U+009B, which some terminals
            // still take as a CSI introducer.
            _ if c.is_control() => {
                let glyph = match c {
                    '\u{7f}' => '␡',
                    '\u{80}'..='\u{9f}' => '␛',
                    _ => char::from_u32(0x2400 + u32::from(c)).unwrap_or('␀'),
                };
                mark(&mut out, options, glyph);
            }
            _ if options.bidi && is_bidi(c) => mark(&mut out, options, '⇄'),
            _ if options.invisibles == Invisibles::All && is_invisible(c) => {
                mark(&mut out, options, '∅');
            }
            _ if options.invisibles == Invisibles::ZeroWidth && is_zero_width(c) => {
                if matches!(c, '\u{200c}' | '\u{200d}') {
                    out.push(c);
                } else {
                    mark(&mut out, options, '∅');
                }
            }
            _ if options.invisibles == Invisibles::ZeroWidthRuns && is_zero_width(c) => {
                // A lone ZWJ/ZWNJ is script shaping (emoji sequences, Persian) if it ends up
                // between two visible characters, settled below once the text around it is
                // clean; any other zero-width character, or a run of them, hides or pads text.
                let lone_joiner = matches!(c, '\u{200c}' | '\u{200d}')
                    && !s
                        .get(next..)
                        .and_then(|rest| rest.chars().next())
                        .is_some_and(is_zero_width);
                if lone_joiner {
                    out.push(c);
                } else {
                    while let Some(n) = s.get(i..).and_then(|rest| rest.chars().next()) {
                        if !is_zero_width(n) {
                            break;
                        }
                        i += n.len_utf8();
                    }
                    mark(&mut out, options, '∅');
                }
            }
            _ => out.push(c),
        }
    }
    if options.invisibles == Invisibles::ZeroWidthRuns && out.contains(['\u{200c}', '\u{200d}']) {
        out = settle_joiners(&out, options);
    }
    if out == s {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(out)
    }
}

/// Keeps a joiner only between two visible characters. Decided on the cleaned text, because the
/// character after a joiner in the input may be an escape sequence that is about to go.
fn settle_joiners(cleaned: &str, options: &Options) -> String {
    let visible = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace() && !is_zero_width(c));
    let mut out = String::with_capacity(cleaned.len());
    let mut chars = cleaned.chars().peekable();
    while let Some(c) = chars.next() {
        if !matches!(c, '\u{200c}' | '\u{200d}')
            || (visible(out.chars().last()) && visible(chars.peek().copied()))
        {
            out.push(c);
        } else {
            mark(&mut out, options, '∅');
        }
    }
    out
}

fn mark(out: &mut String, options: &Options, glyph: char) {
    if options.marks {
        out.push(glyph);
    }
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
}

/// Dropping only the overrides leaves U+202E's quieter relatives (LRM, RLM, ALM) free to
/// reorder a line in any bidi-aware renderer.
fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00ad}' | '\u{061c}'
        | '\u{200b}'..='\u{200f}'
        | '\u{2028}' | '\u{2029}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{206f}'
        | '\u{feff}'
        | '\u{e0000}'..='\u{e007f}')
}

/// The index just past the escape sequence that starts at `at`, which must hold an ESC byte
/// (`0x1b`); always greater than `at`. For scanners that work on bytes, such as one reading a
/// child process's output, which may not be valid UTF-8.
///
/// CSI ends at a final byte (`0x40..=0x7e`), OSC, DCS, SOS, PM and APC at BEL or ST (`ESC \` or
/// U+009C), a two- or three-character escape (`ESC c`, `ESC ( 0`) after its final byte. An
/// unterminated sequence ends before the newline, so it cannot eat the next line, and a CSI
/// ends before any other control byte, so `ESC [ 3 1 ESC [ 0 m` is two sequences. Every end
/// follows an ASCII byte or a newline, so cutting `[at, end)` out of UTF-8 leaves UTF-8.
pub fn skip_escape(bytes: &[u8], at: usize) -> usize {
    let mut j = at + 1;
    match bytes.get(j) {
        Some(b'[') => skip_csi(bytes, j + 1),
        Some(b']' | b'P' | b'X' | b'^' | b'_') => skip_string(bytes, j + 1),
        _ => {
            while matches!(bytes.get(j), Some(0x20..=0x2f)) {
                j += 1;
            }
            if matches!(bytes.get(j), Some(0x30..=0x7e)) {
                j += 1;
            }
            j
        }
    }
}

fn skip_csi(bytes: &[u8], mut i: usize) -> usize {
    while let Some(&b) = bytes.get(i) {
        match b {
            0x00..=0x1f | 0x7f => return i,
            0x40..=0x7e => return i + 1,
            _ => i += 1,
        }
    }
    i
}

fn skip_string(bytes: &[u8], mut i: usize) -> usize {
    while let Some(&b) = bytes.get(i) {
        match (b, bytes.get(i + 1)) {
            (b'\n', _) => return i,
            (0x07, _) => return i + 1,
            (0x1b, Some(b'\\')) | (0xc2, Some(0x9c)) => return i + 2,
            _ => i += 1,
        }
    }
    i
}
