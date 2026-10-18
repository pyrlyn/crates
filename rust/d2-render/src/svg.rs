//! Metadata extracted from a rendered SVG.
//!
//! d2 tags every shape and connection with `<g class="BASE64">`, where the
//! class is the base64 of the element's (XML-escaped) D2 key, e.g. `YQ==` for
//! `a` and `KGEgLSZndDsgYikbMF0=` for `(a -> b)[0]`. [`SvgInfo::parse`] decodes
//! those keys and reads the root size; it is a small tag scanner, not a full
//! XML parser, which is enough for the SVG d2 and the native backend emit.

use std::fmt;
use std::path::Path;

use crate::Result;

/// What kind of diagram element a key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElementKind {
    /// A shape (object), e.g. `grp.c`.
    Shape,
    /// A connection, e.g. `(a -> b)[0]`.
    Connection,
}

/// A shape or connection found in the SVG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgElement {
    /// The D2 key, e.g. `grp.c` or `(a -> b)[0]`.
    pub key: String,
    /// Shape or connection.
    pub kind: ElementKind,
}

/// Size and element keys of a rendered SVG.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SvgInfo {
    /// `viewBox` of the root element: min-x, min-y, width, height.
    pub view_box: Option<[f64; 4]>,
    /// Width in user units (root `width`, else the viewBox width).
    pub width: Option<f64>,
    /// Height in user units.
    pub height: Option<f64>,
    /// `data-d2-version` of the root element.
    pub d2_version: Option<String>,
    /// Every `id` attribute, in document order.
    pub ids: Vec<String>,
    /// Shapes and connections, in document order, without duplicates.
    pub elements: Vec<SvgElement>,
    /// The source hash stamped by this crate, if any.
    pub source_hash: Option<String>,
}

impl SvgInfo {
    /// Parse SVG text.
    pub fn parse(svg: &str) -> SvgInfo {
        let mut info = SvgInfo {
            source_hash: crate::fresh::stamp_in_svg(svg),
            ..SvgInfo::default()
        };
        let mut root_seen = false;
        for tag in Tags::new(svg) {
            if tag.name == "svg" && !root_seen {
                root_seen = true;
                info.view_box = tag.attr("viewBox").and_then(parse_view_box);
                info.width = tag
                    .attr("width")
                    .and_then(parse_len)
                    .or(info.view_box.map(|v| v[2]));
                info.height = tag
                    .attr("height")
                    .and_then(parse_len)
                    .or(info.view_box.map(|v| v[3]));
                info.d2_version = tag.attr("data-d2-version").map(str::to_string);
            }
            if let Some(id) = tag.attr("id") {
                info.ids.push(id.to_string());
            }
            if tag.name == "g" {
                if let Some(class) = tag.attr("class") {
                    // `class="KEY name1 name2"`: D2 appends class names.
                    let first = class.split_ascii_whitespace().next().unwrap_or("");
                    if let Some(key) = decode_key(first) {
                        if !info.elements.iter().any(|e| e.key == key) {
                            let kind = classify(&key);
                            info.elements.push(SvgElement { key, kind });
                        }
                    }
                }
            }
        }
        info
    }

    /// Read and parse an SVG file.
    pub fn from_file(path: &Path) -> Result<SvgInfo> {
        Ok(SvgInfo::parse(&std::fs::read_to_string(path)?))
    }

    /// Keys of all shapes.
    pub fn shapes(&self) -> impl Iterator<Item = &str> {
        self.elements
            .iter()
            .filter(|e| e.kind == ElementKind::Shape)
            .map(|e| e.key.as_str())
    }

    /// Keys of all connections.
    pub fn connections(&self) -> impl Iterator<Item = &str> {
        self.elements
            .iter()
            .filter(|e| e.kind == ElementKind::Connection)
            .map(|e| e.key.as_str())
    }
}

impl fmt::Display for SvgInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.width, self.height) {
            (Some(w), Some(h)) => write!(f, "{w}x{h}")?,
            _ => write!(f, "unknown size")?,
        }
        let shapes = self.shapes().count();
        let conns = self.connections().count();
        write!(f, ", {shapes} shape(s), {conns} connection(s)")
    }
}

fn classify(key: &str) -> ElementKind {
    // A connection key ends in `(src <op> dst)[index]`, possibly prefixed by
    // the container it was declared in (`grp.(a -> b)[0]`).
    if key.ends_with(']') && key.contains('(') {
        ElementKind::Connection
    } else {
        ElementKind::Shape
    }
}

/// Decode a d2 element class into its key, `None` if it is not base64 of a
/// plausible key (d2 also uses plain classes such as `shape`).
pub(crate) fn decode_key(class: &str) -> Option<String> {
    let class = class.trim();
    if class.contains(' ') || class.len() < 4 || class.len() % 4 != 0 {
        return None;
    }
    let bytes = base64_decode(class)?;
    let s = String::from_utf8(bytes).ok()?;
    if s.is_empty() || s.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(unescape_xml(&s))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Standard base64 with padding; `None` on invalid input.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let last = i == bytes.len() / 4 - 1;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (j, &c) in chunk.iter().enumerate() {
            let v = if j >= 4 - pad { 0 } else { val(c)? };
            n = (n << 6) | v;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// Escape text for XML content and attributes.
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape_xml(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_view_box(v: &str) -> Option<[f64; 4]> {
    let nums: Vec<f64> = v
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().ok())
        .collect::<Option<_>>()?;
    (nums.len() == 4).then(|| [nums[0], nums[1], nums[2], nums[3]])
}

fn parse_len(v: &str) -> Option<f64> {
    v.trim().trim_end_matches("px").parse().ok()
}

struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
}

impl<'a> Tag<'a> {
    fn attr(&self, name: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }
}

/// Iterator over start tags, skipping comments, CDATA, `<?...?>` and
/// `<style>`/`<script>` bodies.
struct Tags<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Tags<'a> {
    fn new(s: &'a str) -> Self {
        Tags { s, pos: 0 }
    }
}

impl<'a> Iterator for Tags<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Tag<'a>> {
        loop {
            let rest = &self.s[self.pos..];
            let lt = rest.find('<')?;
            let start = self.pos + lt;
            let after = &self.s[start..];
            let skip_to = |pat: &str| after.find(pat).map(|i| start + i + pat.len());
            if after.starts_with("<!--") {
                self.pos = skip_to("-->").unwrap_or(self.s.len());
                continue;
            }
            if after.starts_with("<![CDATA[") {
                self.pos = skip_to("]]>").unwrap_or(self.s.len());
                continue;
            }
            if after.starts_with("<?") || after.starts_with("<!") || after.starts_with("</") {
                self.pos = skip_to(">").unwrap_or(self.s.len());
                continue;
            }
            let (tag, end) = parse_tag(self.s, start)?;
            self.pos = end;
            if tag.name == "style" || tag.name == "script" {
                let close = format!("</{}", tag.name);
                if let Some(i) = self.s[self.pos..].find(&close) {
                    self.pos += i;
                }
            }
            return Some(tag);
        }
    }
}

/// Parse `<name attr="v" ...>` starting at `start` (which is `<`).
fn parse_tag(s: &str, start: usize) -> Option<(Tag<'_>, usize)> {
    let b = s.as_bytes();
    let mut i = start + 1;
    let name_start = i;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    let name = &s[name_start..i];
    let mut attrs = Vec::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return None;
        }
        if b[i] == b'>' {
            return Some((Tag { name, attrs }, i + 1));
        }
        if b[i] == b'/' {
            i += 1;
            continue;
        }
        let k0 = i;
        while i < b.len() && b[i] != b'=' && b[i] != b'>' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &s[k0..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() {
                return None;
            }
            let q = b[i];
            if q == b'"' || q == b'\'' {
                let v0 = i + 1;
                let len = s[v0..].find(q as char)?;
                attrs.push((key, &s[v0..v0 + len]));
                i = v0 + len + 1;
            } else {
                let v0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                attrs.push((key, &s[v0..i]));
            }
        } else if !key.is_empty() {
            attrs.push((key, ""));
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in ["", "a", "ab", "abc", "grp.c", "(a -&gt; b)[0]"] {
            assert_eq!(
                base64_decode(&base64_encode(s.as_bytes())).unwrap(),
                s.as_bytes()
            );
        }
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert!(base64_decode("Y===").is_none());
    }

    #[test]
    fn parses_d2_like_svg() {
        let svg = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?><svg xmlns="http://www.w3.org/2000/svg" "#,
            r#"data-d2-version="v0.9.0" viewBox="0 0 315 728"><svg class="d2-1 d2-svg" "#,
            r#"width="315" height="728" viewBox="-91 -101 315 728"><style>.a{b:c}</style>"#,
            r#"<g class="YQ=="><g class="shape"></g></g><g class="Z3JwLmM= db hot"></g>"#,
            r#"<g class="KGEgLSZndDsgYilbMF0="></g><marker id="mk-1"/></svg></svg>"#
        );
        let info = SvgInfo::parse(svg);
        assert_eq!(info.view_box, Some([0.0, 0.0, 315.0, 728.0]));
        assert_eq!(info.width, Some(315.0));
        assert_eq!(info.d2_version.as_deref(), Some("v0.9.0"));
        assert_eq!(info.shapes().collect::<Vec<_>>(), ["a", "grp.c"]);
        assert_eq!(info.connections().collect::<Vec<_>>(), ["(a -> b)[0]"]);
        assert_eq!(info.ids, ["mk-1"]);
    }
}
