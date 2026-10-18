// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Approximates the text metrics D2 gets from its bundled Source Sans Pro
// fonts (lib/textmeasure), Copyright 2022 Terrastruct, Inc.

//! Text measurement without loading fonts.

/// Width of one character in em, close to Source Sans Pro Regular.
fn char_em(c: char) -> f64 {
    match c {
        ' ' => 0.2,
        'i' | 'j' | 'l' | '\'' | '.' | ',' | ':' | ';' | '|' | '!' | 'I' => 0.25,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '-' | '/' | '\\' => 0.33,
        'm' | 'w' => 0.78,
        'M' | 'W' => 0.88,
        'A'..='Z' => 0.6,
        '0'..='9' => 0.5,
        'a'..='z' => 0.5,
        '_' | '#' | '$' | '%' | '&' | '@' | '+' | '=' | '<' | '>' | '?' | '*' | '~' | '^' => 0.55,
        c if c.is_ascii() => 0.5,
        // CJK and other wide scripts.
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.6,
    }
}

/// Line height as a multiple of the font size.
pub const LINE_HEIGHT: f64 = 1.3;

/// `(width, height)` of possibly multi-line `text` at `font_size` px.
pub fn measure(text: &str, font_size: f64, bold: bool) -> (f64, f64) {
    let factor = if bold { 1.06 } else { 1.0 };
    let mut width: f64 = 0.0;
    let mut lines = 0usize;
    for line in text.split('\n') {
        lines += 1;
        let w: f64 = line.chars().map(char_em).sum::<f64>() * font_size * factor;
        width = width.max(w);
    }
    (
        width.ceil(),
        (lines.max(1) as f64 * font_size * LINE_HEIGHT).ceil(),
    )
}

/// `(width, height)` in a monospace font (0.6 em per character).
pub fn measure_mono(text: &str, font_size: f64) -> (f64, f64) {
    let lines: Vec<&str> = text.split('\n').collect();
    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    (
        (cols as f64 * 0.6 * font_size).ceil(),
        (lines.len().max(1) as f64 * font_size * LINE_HEIGHT).ceil(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_with_text() {
        let (w1, h1) = measure("a", 16.0, false);
        let (w2, _) = measure("alpha beta", 16.0, false);
        let (_, h2) = measure("a\nb", 16.0, false);
        assert!(w2 > w1);
        assert!(h2 > h1 * 1.9);
    }
}
