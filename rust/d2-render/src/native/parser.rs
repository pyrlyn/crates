// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Rust rewrite of parts of D2's parser (d2parser), Copyright 2022
// Terrastruct, Inc. Error messages follow D2's wording so diagnostics from
// the native backend read like the CLI's.

//! Parser for the subset of D2 the native backend understands.

use crate::diagnostic::Diagnostic;

/// 1-based source position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    /// Line.
    pub line: u32,
    /// Column, in characters.
    pub col: u32,
}

/// One key segment, e.g. `a` in `a.b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    /// The text, unquoted.
    pub text: String,
    /// Written in quotes (never a keyword then).
    pub quoted: bool,
    /// Where it starts.
    pub pos: Pos,
}

/// `a.b.c`.
pub type KeyPath = Vec<Seg>;

/// A connection operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeOp {
    /// `->`
    Forward,
    /// `<-`
    Backward,
    /// `<->`
    Both,
    /// `--`
    Undirected,
}

impl EdgeOp {
    /// As written in D2.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeOp::Forward => "->",
            EdgeOp::Backward => "<-",
            EdgeOp::Both => "<->",
            EdgeOp::Undirected => "--",
        }
    }
}

/// A scalar value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    /// The text, unquoted / unescaped.
    pub text: String,
    /// Where it starts.
    pub pos: Pos,
    /// Came from a `|...|` block string.
    pub block: bool,
    /// The quote character, for quoted strings (`'...'` skips `${var}`
    /// substitution, like in D2).
    pub quote: Option<char>,
    /// Items of an array value `[a; b]`.
    pub items: Option<Vec<String>>,
}

impl Value {
    /// A plain unquoted value.
    pub fn plain(text: impl Into<String>, pos: Pos) -> Self {
        Value {
            text: text.into(),
            pos,
            block: false,
            quote: None,
            items: None,
        }
    }
}

/// One statement: `key: value { map }` or `a -> b -> c: label { map }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    /// Start of the statement.
    pub pos: Pos,
    /// Key paths; more than one means a connection chain.
    pub keys: Vec<KeyPath>,
    /// Operators between `keys` (`keys.len() - 1` of them).
    pub ops: Vec<EdgeOp>,
    /// The scalar after `:`.
    pub value: Option<Value>,
    /// The nested map.
    pub map: Option<Map>,
}

impl Stmt {
    /// A connection statement.
    pub fn is_edge(&self) -> bool {
        !self.ops.is_empty()
    }
}

/// `{ ... }` or the whole file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Map {
    /// Statements in order.
    pub stmts: Vec<Stmt>,
}

/// Parse a whole file. All syntax errors are collected.
pub fn parse(src: &str) -> Result<Map, Vec<Diagnostic>> {
    let mut p = Parser {
        s: src.chars().collect(),
        i: 0,
        line: 1,
        col: 1,
        errors: Vec::new(),
    };
    let m = p.parse_map(None);
    if p.errors.is_empty() {
        Ok(m)
    } else {
        Err(p.errors)
    }
}

struct Parser {
    s: Vec<char>,
    i: usize,
    line: u32,
    col: u32,
    errors: Vec<Diagnostic>,
}

impl Parser {
    fn pos(&self) -> Pos {
        Pos {
            line: self.line,
            col: self.col,
        }
    }

    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.s.get(self.i + n).copied()
    }

    fn starts_with(&self, pat: &str) -> bool {
        pat.chars()
            .enumerate()
            .all(|(k, c)| self.peek_at(k) == Some(c))
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn err(&mut self, pos: Pos, msg: impl Into<String>) {
        self.errors.push(Diagnostic::error(pos.line, pos.col, msg));
    }

    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
            self.bump();
        }
    }

    fn skip_comment(&mut self) -> bool {
        if self.starts_with("\"\"\"") {
            // Block comment: """ ... """
            for _ in 0..3 {
                self.bump();
            }
            while self.peek().is_some() && !self.starts_with("\"\"\"") {
                self.bump();
            }
            for _ in 0..3 {
                self.bump();
            }
            return true;
        }
        if self.peek() == Some('#') {
            while self.peek().is_some_and(|c| c != '\n') {
                self.bump();
            }
            return true;
        }
        false
    }

    /// Skip whitespace, newlines, `;` and comments between statements.
    fn skip_between(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r' | '\n' | ';') => {
                    self.bump();
                }
                Some('#') => {
                    self.skip_comment();
                }
                Some('"') if self.starts_with("\"\"\"") => {
                    self.skip_comment();
                }
                _ => return,
            }
        }
    }

    fn parse_map(&mut self, open: Option<Pos>) -> Map {
        let mut m = Map::default();
        loop {
            self.skip_between();
            match self.peek() {
                None => {
                    if let Some(p) = open {
                        self.err(p, "maps must be terminated with }");
                    }
                    return m;
                }
                Some('}') => {
                    let p = self.pos();
                    self.bump();
                    if open.is_some() {
                        return m;
                    }
                    self.err(p, "unexpected map termination character } in file map");
                }
                _ => {
                    let before = self.i;
                    if let Some(st) = self.parse_stmt() {
                        m.stmts.push(st);
                    }
                    if self.i == before {
                        // Never loop without progress.
                        self.bump();
                    }
                }
            }
        }
    }

    fn at_edge_op(&self) -> Option<(EdgeOp, usize)> {
        if self.starts_with("<->") {
            Some((EdgeOp::Both, 3))
        } else if self.starts_with("->") {
            Some((EdgeOp::Forward, 2))
        } else if self.starts_with("<-") {
            Some((EdgeOp::Backward, 2))
        } else if self.starts_with("--") {
            Some((EdgeOp::Undirected, 2))
        } else {
            None
        }
    }

    fn at_stmt_end(&self) -> bool {
        matches!(self.peek(), None | Some('\n' | ';' | '}' | '#'))
    }

    /// Skip the rest of a broken statement.
    fn recover(&mut self) {
        while !matches!(self.peek(), None | Some('\n' | ';' | '}')) {
            self.bump();
        }
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let start = self.pos();
        let mut keys = Vec::new();
        let mut ops = Vec::new();
        self.skip_inline_ws();
        if self.peek() == Some('(') {
            self.err(
                start,
                "connection references like (a -> b)[0] are not supported by the native backend",
            );
            self.recover();
            return None;
        }
        if self.starts_with("...$") {
            self.err(
                start,
                "variable spreads are not supported by the native backend",
            );
            self.recover();
            return None;
        }
        if self.starts_with("...@") || self.peek() == Some('@') {
            self.err(start, "imports are not supported by the native backend");
            self.recover();
            return None;
        }
        keys.push(self.parse_key_path()?);
        loop {
            self.skip_inline_ws();
            let Some((op, n)) = self.at_edge_op() else {
                break;
            };
            for _ in 0..n {
                self.bump();
            }
            ops.push(op);
            self.skip_inline_ws();
            let k = self.parse_key_path()?;
            if k.is_empty() {
                self.err(start, "connection missing destination");
                self.recover();
                return None;
            }
            keys.push(k);
        }
        if keys[0].is_empty() {
            if !ops.is_empty() {
                self.err(start, "connection missing source");
            } else {
                let c = self.peek().unwrap_or(' ');
                self.err(start, format!("unexpected text {c:?}"));
            }
            self.recover();
            return None;
        }
        self.skip_inline_ws();
        let mut value = None;
        let mut map = None;
        if self.peek() == Some(':') {
            let colon = self.pos();
            self.bump();
            self.skip_inline_ws();
            if self.peek() == Some('{') {
                // map below
            } else if self.at_stmt_end() {
                self.err(colon, "missing value after colon");
                self.recover();
                return None;
            } else {
                value = Some(self.parse_value()?);
                self.skip_inline_ws();
            }
        }
        if self.peek() == Some('{') {
            let open = self.pos();
            self.bump();
            map = Some(self.parse_map(Some(open)));
            self.skip_inline_ws();
        }
        self.skip_comment();
        if !self.at_stmt_end() {
            let p = self.pos();
            self.err(p, "unexpected text after statement");
            self.recover();
            return None;
        }
        Some(Stmt {
            pos: start,
            keys,
            ops,
            value,
            map,
        })
    }

    fn parse_key_path(&mut self) -> Option<KeyPath> {
        let mut segs = Vec::new();
        loop {
            self.skip_inline_ws();
            let pos = self.pos();
            match self.peek() {
                Some(q @ ('"' | '\'')) => {
                    let text = self.parse_quoted(q)?;
                    segs.push(Seg {
                        text,
                        quoted: true,
                        pos,
                    });
                }
                _ => {
                    let mut text = String::new();
                    while let Some(c) = self.peek() {
                        if matches!(c, '.' | ':' | ';' | '{' | '}' | '\n' | '#')
                            || self.at_edge_op().is_some()
                        {
                            break;
                        }
                        text.push(c);
                        self.bump();
                    }
                    let text = text.trim().to_string();
                    if text.is_empty() {
                        if segs.is_empty() {
                            return Some(segs);
                        }
                        self.err(pos, "empty key segment");
                        self.recover();
                        return None;
                    }
                    if text.contains('*') {
                        self.err(pos, "globs are not supported by the native backend");
                        self.recover();
                        return None;
                    }
                    segs.push(Seg {
                        text,
                        quoted: false,
                        pos,
                    });
                }
            }
            self.skip_inline_ws();
            if self.peek() == Some('.') {
                self.bump();
            } else {
                return Some(segs);
            }
        }
    }

    fn parse_quoted(&mut self, q: char) -> Option<String> {
        let start = self.pos();
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some('\n') => {
                    self.err(start, format!("unterminated string literal; expected {q}"));
                    return None;
                }
                Some('\\') if q == '"' => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(c) => out.push(c),
                    None => {}
                },
                Some(c) if c == q => return Some(out),
                Some(c) => out.push(c),
            }
        }
    }

    fn parse_value(&mut self) -> Option<Value> {
        let pos = self.pos();
        match self.peek() {
            Some(q @ ('"' | '\'')) => {
                let text = self.parse_quoted(q)?;
                Some(Value {
                    quote: Some(q),
                    ..Value::plain(text, pos)
                })
            }
            Some('|') => self.parse_block(pos),
            Some('[') => self.parse_array(pos),
            _ => {
                let mut text = String::new();
                while let Some(c) = self.peek() {
                    // `${var}` substitutions are part of the value.
                    if c == '$' && self.peek_at(1) == Some('{') {
                        while let Some(c) = self.peek().filter(|c| *c != '\n') {
                            text.push(c);
                            self.bump();
                            if c == '}' {
                                break;
                            }
                        }
                        continue;
                    }
                    if matches!(c, '\n' | ';' | '{' | '}') {
                        break;
                    }
                    if c == '#' && text.ends_with([' ', '\t']) {
                        break;
                    }
                    text.push(c);
                    self.bump();
                }
                Some(Value::plain(text.trim(), pos))
            }
        }
    }

    /// `[a; b; "c d"]`.
    fn parse_array(&mut self, pos: Pos) -> Option<Value> {
        self.bump();
        let mut items = Vec::new();
        loop {
            while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n' | ';')) {
                self.bump();
            }
            match self.peek() {
                None => {
                    self.err(pos, "arrays must be terminated with ]");
                    return None;
                }
                Some(']') => {
                    self.bump();
                    break;
                }
                Some(q @ ('"' | '\'')) => items.push(self.parse_quoted(q)?),
                Some(_) => {
                    let mut t = String::new();
                    while let Some(c) = self.peek() {
                        if matches!(c, ';' | ']' | '\n') {
                            break;
                        }
                        t.push(c);
                        self.bump();
                    }
                    items.push(t.trim().to_string());
                }
            }
        }
        Some(Value {
            text: items.join("; "),
            items: Some(items),
            ..Value::plain("", pos)
        })
    }

    /// `|md text|`, `||x||`, `|||...|||` (the tag is ignored).
    fn parse_block(&mut self, pos: Pos) -> Option<Value> {
        let mut pipes = 0;
        while self.peek() == Some('|') {
            pipes += 1;
            self.bump();
        }
        while self.peek().is_some_and(|c| !c.is_whitespace()) {
            self.bump();
        }
        let close: String = "|".repeat(pipes);
        let mut text = String::new();
        loop {
            if self.peek().is_none() {
                self.err(pos, format!("block string must be terminated with {close}"));
                return None;
            }
            if self.starts_with(&close) {
                for _ in 0..pipes {
                    self.bump();
                }
                break;
            }
            text.push(self.bump().unwrap_or_default());
        }
        let lines: Vec<&str> = text.lines().collect();
        let indent = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        let text = lines
            .iter()
            .map(|l| {
                if l.len() >= indent {
                    &l[indent..]
                } else {
                    l.trim_start()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        Some(Value {
            block: true,
            ..Value::plain(text, pos)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errs(src: &str) -> Vec<String> {
        parse(src)
            .unwrap_err()
            .into_iter()
            .map(|d| d.to_string())
            .collect()
    }

    #[test]
    fn basic() {
        let m = parse("a: Alpha\nb: {shape: circle}\na -> b -> c: go; # c\n").unwrap();
        assert_eq!(m.stmts.len(), 3);
        assert_eq!(m.stmts[0].value.as_ref().unwrap().text, "Alpha");
        assert_eq!(m.stmts[1].map.as_ref().unwrap().stmts.len(), 1);
        assert_eq!(m.stmts[2].keys.len(), 3);
        assert_eq!(m.stmts[2].ops, [EdgeOp::Forward, EdgeOp::Forward]);
    }

    #[test]
    fn quoted_and_dots() {
        let m = parse("\"a.b\".c -- 'd e'\n").unwrap();
        let s = &m.stmts[0];
        assert_eq!(s.keys[0][0].text, "a.b");
        assert!(s.keys[0][0].quoted);
        assert_eq!(s.keys[0][1].text, "c");
        assert_eq!(s.keys[1][0].text, "d e");
    }

    #[test]
    fn block_string() {
        let m = parse("x: |md\n  # Title\n  body\n|\n").unwrap();
        assert_eq!(m.stmts[0].value.as_ref().unwrap().text, "# Title\nbody");
    }

    #[test]
    fn errors_match_d2_positions() {
        assert_eq!(
            errs("a -> b\nx: {\n  shape: circle\n}\nc: {\n"),
            ["5:4: maps must be terminated with }"]
        );
        assert_eq!(
            errs("a\nb\na -> \n"),
            ["3:1: connection missing destination"]
        );
        assert_eq!(
            errs("z.style.fill: \n"),
            ["1:13: missing value after colon"]
        );
    }
}
