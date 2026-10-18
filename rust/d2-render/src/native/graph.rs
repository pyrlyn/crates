// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Rust rewrite of parts of D2's compiler and graph model (d2compiler,
// d2graph, d2ast keyword tables), Copyright 2022 Terrastruct, Inc.

//! Turns the parsed AST into a graph of objects and connections.
//!
//! Handles scoped `vars` (with `${name}` substitution and `d2-config`),
//! scoped `classes`, `near` constants, icons, tooltips, links, grids and
//! arrowheads on top of shapes, labels, containers, connections and styles.

use std::collections::HashMap;

use super::parser::{EdgeOp, KeyPath, Map, Pos, Seg, Stmt, Value};
use crate::diagnostic::Diagnostic;

/// Shapes D2 knows (d2target).
pub const SHAPES: &[&str] = &[
    "rectangle",
    "square",
    "page",
    "parallelogram",
    "document",
    "cylinder",
    "queue",
    "package",
    "step",
    "callout",
    "stored_data",
    "person",
    "c4-person",
    "diamond",
    "oval",
    "circle",
    "hexagon",
    "cloud",
    "text",
    "code",
    "class",
    "sql_table",
    "image",
    "sequence_diagram",
    "hierarchy",
];

/// Shapes D2 knows that the native renderer does not draw; they fall back
/// to a rectangle with a warning.
pub const UNSUPPORTED_SHAPES: &[&str] = &["class", "sql_table", "sequence_diagram"];

/// Arrowhead shapes D2 knows.
pub const ARROWHEADS: &[&str] = &[
    "triangle",
    "arrow",
    "diamond",
    "circle",
    "box",
    "cf-one",
    "cf-one-required",
    "cf-many",
    "cf-many-required",
    "cross",
];

/// `near` constants.
pub const NEAR_CONSTANTS: &[&str] = &[
    "top-left",
    "top-center",
    "top-right",
    "center-left",
    "center-right",
    "bottom-left",
    "bottom-center",
    "bottom-right",
];

const SIMPLE_KEYWORDS: &[&str] = &[
    "label",
    "shape",
    "icon",
    "constraint",
    "tooltip",
    "link",
    "near",
    "width",
    "height",
    "direction",
    "top",
    "left",
    "grid-rows",
    "grid-columns",
    "grid-gap",
    "vertical-gap",
    "horizontal-gap",
    "class",
    "vars",
];

const COMPOSITE_KEYWORDS: &[&str] = &[
    "source-arrowhead",
    "target-arrowhead",
    "classes",
    "constraint",
    "label",
    "icon",
    "tooltip",
];

const STYLE_KEYWORDS: &[&str] = &[
    "opacity",
    "stroke",
    "fill",
    "fill-pattern",
    "stroke-width",
    "stroke-dash",
    "border-radius",
    "font",
    "font-size",
    "font-color",
    "bold",
    "italic",
    "underline",
    "text-transform",
    "shadow",
    "multiple",
    "double-border",
    "3d",
    "animated",
    "filled",
];

fn is_reserved(s: &Seg) -> bool {
    !s.quoted
        && (s.text == "style"
            || SIMPLE_KEYWORDS.contains(&s.text.as_str())
            || COMPOSITE_KEYWORDS.contains(&s.text.as_str()))
}

fn is_style_keyword(s: &Seg) -> bool {
    !s.quoted && STYLE_KEYWORDS.contains(&s.text.as_str())
}

/// Visual style of a shape or connection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    /// `style.fill` (a colour or a theme code such as `B4`).
    pub fill: Option<String>,
    /// `style.stroke`.
    pub stroke: Option<String>,
    /// `style.stroke-width`.
    pub stroke_width: Option<f64>,
    /// `style.stroke-dash`.
    pub stroke_dash: Option<f64>,
    /// `style.opacity`.
    pub opacity: Option<f64>,
    /// `style.border-radius`.
    pub border_radius: Option<f64>,
    /// `style.font-size`.
    pub font_size: Option<f64>,
    /// `style.font-color`.
    pub font_color: Option<String>,
    /// `style.font` (`mono` switches to a monospace family).
    pub font: Option<String>,
    /// `style.bold`.
    pub bold: Option<bool>,
    /// `style.italic`.
    pub italic: Option<bool>,
    /// `style.underline`.
    pub underline: Option<bool>,
    /// `style.text-transform`.
    pub text_transform: Option<String>,
    /// `style.shadow`.
    pub shadow: Option<bool>,
    /// `style.double-border`.
    pub double_border: Option<bool>,
    /// `style.multiple`.
    pub multiple: Option<bool>,
    /// `style.3d`.
    pub three_d: Option<bool>,
    /// `style.animated` (connections).
    pub animated: Option<bool>,
    /// `style.filled` (arrowheads).
    pub filled: Option<bool>,
}

/// Layout direction of a board or container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Top to bottom (default).
    #[default]
    Down,
    /// Bottom to top.
    Up,
    /// Left to right.
    Right,
    /// Right to left.
    Left,
}

/// A `near` constant for a root-level shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Near {
    /// `top-left`
    TopLeft,
    /// `top-center`
    TopCenter,
    /// `top-right`
    TopRight,
    /// `center-left`
    CenterLeft,
    /// `center-right`
    CenterRight,
    /// `bottom-left`
    BottomLeft,
    /// `bottom-center`
    BottomCenter,
    /// `bottom-right`
    BottomRight,
}

impl Near {
    fn parse(s: &str) -> Option<Near> {
        Some(match s {
            "top-left" => Near::TopLeft,
            "top-center" => Near::TopCenter,
            "top-right" => Near::TopRight,
            "center-left" => Near::CenterLeft,
            "center-right" => Near::CenterRight,
            "bottom-left" => Near::BottomLeft,
            "bottom-center" => Near::BottomCenter,
            "bottom-right" => Near::BottomRight,
            _ => return None,
        })
    }
}

/// Arrowhead shapes the renderer draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArrowShape {
    /// Filled triangle (default).
    #[default]
    Triangle,
    /// Narrow arrow.
    Arrow,
    /// Diamond (unfilled unless `style.filled: true`).
    Diamond,
    /// Circle (unfilled unless `style.filled: true`).
    Circle,
    /// Square (unfilled unless `style.filled: true`).
    Box,
    /// An X.
    Cross,
    /// Crow's foot: one.
    CfOne,
    /// Crow's foot: exactly one.
    CfOneRequired,
    /// Crow's foot: many.
    CfMany,
    /// Crow's foot: one or more.
    CfManyRequired,
}

impl ArrowShape {
    fn parse(s: &str) -> Option<ArrowShape> {
        Some(match s {
            "triangle" => ArrowShape::Triangle,
            "arrow" => ArrowShape::Arrow,
            "diamond" => ArrowShape::Diamond,
            "circle" => ArrowShape::Circle,
            "box" => ArrowShape::Box,
            "cross" => ArrowShape::Cross,
            "cf-one" => ArrowShape::CfOne,
            "cf-one-required" => ArrowShape::CfOneRequired,
            "cf-many" => ArrowShape::CfMany,
            "cf-many-required" => ArrowShape::CfManyRequired,
            _ => return None,
        })
    }
}

/// One end's arrowhead.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arrowhead {
    /// Shape.
    pub shape: ArrowShape,
    /// `style.filled`; `None` uses the shape's default.
    pub filled: Option<bool>,
    /// Label drawn next to the arrowhead.
    pub label: Option<String>,
    /// Whether the connection's direction draws this arrowhead (`->` draws
    /// only the target one); labels are shown either way, as in D2.
    pub drawn: bool,
}

/// Board-level settings from `vars.d2-config`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// `theme-id`.
    pub theme_id: Option<i64>,
    /// `dark-theme-id`.
    pub dark_theme_id: Option<i64>,
    /// `pad`.
    pub pad: Option<f64>,
    /// `sketch`.
    pub sketch: Option<bool>,
    /// `layout-engine`.
    pub layout_engine: Option<String>,
    /// `center`.
    pub center: Option<bool>,
}

/// A shape. Index 0 of [`Graph::objects`] is the root board.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Object {
    /// Last key segment.
    pub id: String,
    /// Full key, e.g. `grp.c`.
    pub abs: String,
    /// Parent index (root is its own parent).
    pub parent: usize,
    /// Children in declaration order.
    pub children: Vec<usize>,
    /// Explicit label (`None` means the id).
    pub label: Option<String>,
    /// Label came from a block string.
    pub label_block: bool,
    /// Shape name (`rectangle` when unset).
    pub shape: String,
    /// `shape` was set explicitly.
    pub shape_set: bool,
    /// Style.
    pub style: Style,
    /// `direction`.
    pub direction: Option<Direction>,
    /// `width`.
    pub width: Option<f64>,
    /// `height`.
    pub height: Option<f64>,
    /// `icon` (URL or path, referenced from the SVG).
    pub icon: Option<String>,
    /// `tooltip` (SVG `<title>`).
    pub tooltip: Option<String>,
    /// `link` (SVG `<a href>`).
    pub link: Option<String>,
    /// Class names (`class: x` / `[a; b]`), written to the SVG like D2.
    pub classes: Vec<String>,
    /// `near` constant (root-level shapes only).
    pub near: Option<Near>,
    /// `grid-rows`.
    pub grid_rows: Option<usize>,
    /// `grid-columns`.
    pub grid_columns: Option<usize>,
    /// `grid-gap`.
    pub grid_gap: Option<f64>,
    /// `vertical-gap`.
    pub vertical_gap: Option<f64>,
    /// `horizontal-gap`.
    pub horizontal_gap: Option<f64>,
    /// Depth: root is 0.
    pub level: usize,
}

impl Object {
    /// Label text to draw.
    pub fn label_text(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }

    /// Has children.
    pub fn is_container(&self) -> bool {
        !self.children.is_empty()
    }

    /// Lays its children out as a grid.
    pub fn is_grid(&self) -> bool {
        self.grid_rows.is_some() || self.grid_columns.is_some()
    }
}

/// A connection.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// Source object.
    pub src: usize,
    /// Destination object.
    pub dst: usize,
    /// Arrowhead at the source.
    pub src_head: Option<Arrowhead>,
    /// Arrowhead at the destination.
    pub dst_head: Option<Arrowhead>,
    /// Label.
    pub label: Option<String>,
    /// Style.
    pub style: Style,
    /// D2 key, e.g. `grp.(a -> b)[0]`.
    pub key: String,
    /// `tooltip`.
    pub tooltip: Option<String>,
    /// `link`.
    pub link: Option<String>,
    /// Class names (`class: x` / `[a; b]`), written to the SVG like D2.
    pub classes: Vec<String>,
}

/// A compiled board.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Graph {
    /// Objects; 0 is the root.
    pub objects: Vec<Object>,
    /// Connections in declaration order.
    pub edges: Vec<Edge>,
    /// Non-fatal notes about unsupported features.
    pub warnings: Vec<String>,
    /// `vars.d2-config` settings.
    pub config: Config,
}

impl Graph {
    fn new() -> Self {
        Graph {
            objects: vec![Object {
                shape: "rectangle".into(),
                ..Object::default()
            }],
            ..Graph::default()
        }
    }

    /// Ancestors of `i` from its parent up to (and including) the root.
    pub fn ancestors(&self, mut i: usize) -> Vec<usize> {
        let mut v = Vec::new();
        while i != 0 {
            i = self.objects[i].parent;
            v.push(i);
        }
        v
    }

    /// `a` is `b` or contains it.
    pub fn contains(&self, a: usize, b: usize) -> bool {
        a == b || a == 0 || self.ancestors(b).contains(&a)
    }
}

/// Compile a parsed file.
pub fn compile(map: &Map) -> Result<Graph, Vec<Diagnostic>> {
    let mut c = Compiler {
        g: Graph::new(),
        errors: Vec::new(),
        edge_counts: HashMap::new(),
        warned: Vec::new(),
        vars: Vec::new(),
        classes: Vec::new(),
        pending_classes: Vec::new(),
        pending_near: Vec::new(),
        weak: false,
    };
    c.map(0, map);
    c.apply_classes();
    c.check_near();
    if c.errors.is_empty() {
        Ok(c.g)
    } else {
        Err(c.errors)
    }
}

#[derive(Debug, Clone, Copy)]
enum Target {
    Object(usize),
    Edge(usize),
}

struct Compiler {
    g: Graph,
    errors: Vec<Diagnostic>,
    edge_counts: HashMap<String, usize>,
    warned: Vec<String>,
    /// Scoped variables, innermost last; keys are dotted paths.
    vars: Vec<HashMap<String, String>>,
    /// Scoped class definitions, innermost last.
    classes: Vec<HashMap<String, Map>>,
    /// Classes to apply after compilation (explicit fields win).
    pending_classes: Vec<(Target, Map)>,
    /// `near` values that are not constants, checked once all objects exist.
    pending_near: Vec<(usize, Value)>,
    /// When set, setters only fill fields that are still unset (classes).
    weak: bool,
}

fn put<T>(weak: bool, slot: &mut Option<T>, v: T) {
    if !weak || slot.is_none() {
        *slot = Some(v);
    }
}

fn key_string(segs: &[String]) -> String {
    segs.iter()
        .map(|s| {
            if s.is_empty() || s.contains(['.', ':', ';', '{', '}', '"', '\'', '#']) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn keyword_stmt<'a>(st: &'a Stmt, name: &str) -> Option<&'a [Seg]> {
    let k = st.keys.first()?;
    (!st.is_edge() && !k[0].quoted && k[0].text == name).then(|| &k[1..])
}

impl Compiler {
    fn err(&mut self, pos: Pos, msg: impl Into<String>) {
        self.errors.push(Diagnostic::error(pos.line, pos.col, msg));
    }

    fn warn_once(&mut self, msg: String) {
        if !self.warned.contains(&msg) {
            self.warned.push(msg.clone());
            self.g.warnings.push(msg);
        }
    }

    // ---- vars -------------------------------------------------------

    fn lookup_var(&self, name: &str) -> Option<String> {
        self.vars.iter().rev().find_map(|f| f.get(name).cloned())
    }

    fn subst_text(&mut self, text: &str, pos: Pos) -> Option<String> {
        if !text.contains("${") {
            return Some(text.to_string());
        }
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find("${") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[i..]);
                return Some(out);
            };
            let name = after[..end].trim();
            match self.lookup_var(name) {
                Some(v) => out.push_str(&v),
                None => {
                    self.err(pos, format!("could not resolve variable {name:?}"));
                    return None;
                }
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// `${var}` substitution, skipped for single-quoted strings.
    fn subst(&mut self, v: &Value) -> Option<Value> {
        if v.quote == Some('\'') {
            return Some(v.clone());
        }
        let mut out = v.clone();
        out.text = self.subst_text(&v.text, v.pos)?;
        if let Some(items) = &v.items {
            let mut new = Vec::new();
            for it in items {
                new.push(self.subst_text(it, v.pos)?);
            }
            out.items = Some(new);
        }
        Some(out)
    }

    fn collect_vars(&mut self, m: &Map, prefix: &str) {
        for st in &m.stmts {
            if st.is_edge() {
                continue;
            }
            let name: Vec<&str> = st.keys[0].iter().map(|s| s.text.as_str()).collect();
            let full = if prefix.is_empty() {
                name.join(".")
            } else {
                format!("{prefix}.{}", name.join("."))
            };
            if full == "d2-config" || full.starts_with("d2-config.") {
                self.config_entry(st, &full);
                continue;
            }
            if let Some(v) = &st.value {
                if let Some(v) = self.subst(v) {
                    if let Some(top) = self.vars.last_mut() {
                        top.insert(full.clone(), v.text);
                    }
                }
            }
            if let Some(inner) = &st.map {
                self.collect_vars(inner, &full);
            }
        }
    }

    fn config_entry(&mut self, st: &Stmt, full: &str) {
        if let Some(inner) = &st.map {
            for s in &inner.stmts {
                if !s.is_edge() {
                    let name: Vec<&str> = s.keys[0].iter().map(|x| x.text.as_str()).collect();
                    self.config_entry(s, &format!("{full}.{}", name.join(".")));
                }
            }
            return;
        }
        let Some(v) = st.value.clone() else { return };
        let Some(v) = self.subst(&v) else { return };
        let cfg = &mut self.g.config;
        match full {
            "d2-config.theme-id" => cfg.theme_id = v.text.parse().ok(),
            "d2-config.dark-theme-id" => cfg.dark_theme_id = v.text.parse().ok(),
            "d2-config.pad" => cfg.pad = v.text.parse().ok(),
            "d2-config.sketch" => cfg.sketch = Some(v.text == "true"),
            "d2-config.center" => cfg.center = Some(v.text == "true"),
            "d2-config.layout-engine" => cfg.layout_engine = Some(v.text.clone()),
            other => {
                let msg = format!("{other} is ignored by the native backend");
                self.warn_once(msg);
            }
        }
    }

    // ---- maps and statements ----------------------------------------

    fn map(&mut self, scope: usize, map: &Map) {
        self.vars.push(HashMap::new());
        for st in &map.stmts {
            if let Some(rest) = keyword_stmt(st, "vars") {
                if rest.is_empty() {
                    if let Some(m) = &st.map {
                        self.collect_vars(m, "");
                    }
                } else {
                    let wrapped = Map {
                        stmts: vec![Stmt {
                            keys: vec![rest.to_vec()],
                            ..st.clone()
                        }],
                    };
                    self.collect_vars(&wrapped, "");
                }
            }
        }
        let mut frame = HashMap::new();
        for st in &map.stmts {
            if let Some(rest) = keyword_stmt(st, "classes") {
                let defs: Vec<(String, Map)> = match (rest.first(), &st.map) {
                    (None, Some(m)) => m
                        .stmts
                        .iter()
                        .filter(|s| !s.is_edge() && s.keys[0].len() == 1)
                        .map(|s| (s.keys[0][0].text.clone(), s.map.clone().unwrap_or_default()))
                        .collect(),
                    (Some(name), Some(m)) if rest.len() == 1 => {
                        vec![(name.text.clone(), m.clone())]
                    }
                    _ => {
                        self.err(st.pos, "classes must be a map of class definitions");
                        Vec::new()
                    }
                };
                for (name, m) in defs {
                    let m = self.subst_map(&m);
                    frame.insert(name, m);
                }
            }
        }
        self.classes.push(frame);
        for st in &map.stmts {
            if keyword_stmt(st, "vars").is_some() || keyword_stmt(st, "classes").is_some() {
                continue;
            }
            if let Some(board) = ["layers", "scenarios", "steps"]
                .into_iter()
                .find(|b| keyword_stmt(st, b).is_some())
            {
                self.warn_once(format!(
                    "{board} are not supported by the native backend; only the root board is drawn"
                ));
                continue;
            }
            self.stmt(scope, st);
        }
        self.classes.pop();
        self.vars.pop();
    }

    /// Substitute variables in every value of a class body.
    fn subst_map(&mut self, m: &Map) -> Map {
        let mut out = m.clone();
        for st in &mut out.stmts {
            if let Some(v) = &st.value {
                if let Some(v) = self.subst(v) {
                    st.value = Some(v);
                }
            }
            if let Some(inner) = &st.map {
                st.map = Some(self.subst_map(inner));
            }
        }
        out
    }

    fn lookup_class(&self, name: &str) -> Option<Map> {
        self.classes.iter().rev().find_map(|f| f.get(name).cloned())
    }

    fn queue_classes(&mut self, target: Target, v: &Value) {
        let names = v.items.clone().unwrap_or_else(|| vec![v.text.clone()]);
        if !self.weak {
            let slot = match target {
                Target::Object(i) => &mut self.g.objects[i].classes,
                Target::Edge(i) => &mut self.g.edges[i].classes,
            };
            slot.clone_from(&names);
        }
        for n in names {
            match self.lookup_class(&n) {
                Some(m) => self.pending_classes.push((target, m)),
                None => self.warn_once(format!("class {n:?} is not defined")),
            }
        }
    }

    fn apply_classes(&mut self) {
        // Explicit fields win, and with several classes the last one wins,
        // so apply in reverse and only fill what is still unset.
        for _round in 0..4 {
            let pending = std::mem::take(&mut self.pending_classes);
            if pending.is_empty() {
                break;
            }
            self.weak = true;
            for (target, m) in pending.into_iter().rev() {
                match target {
                    Target::Object(o) => {
                        for st in &m.stmts {
                            self.stmt(o, st);
                        }
                    }
                    Target::Edge(e) => self.edge_map(e, &m),
                }
            }
            self.weak = false;
        }
    }

    fn check_near(&mut self) {
        for (obj, v) in std::mem::take(&mut self.pending_near) {
            let exists = self
                .g
                .objects
                .iter()
                .any(|o| o.abs.eq_ignore_ascii_case(&v.text));
            if exists {
                let _ = obj;
                self.warn_once(
                    "near an object is not supported by the native backend and was ignored".into(),
                );
            } else {
                self.err(
                    v.pos,
                    format!(
                        "near key {:?} must be the absolute path to a shape or one of the following constants: {}",
                        v.text,
                        NEAR_CONSTANTS.join(", ")
                    ),
                );
            }
        }
    }

    fn stmt(&mut self, scope: usize, st: &Stmt) {
        if st.is_edge() {
            self.edge_stmt(scope, st);
            return;
        }
        let path = &st.keys[0];
        let kw = path.iter().position(is_reserved);
        if let Some(s) = path.iter().find(|s| is_style_keyword(s)) {
            if !path.iter().any(|s| !s.quoted && s.text == "style") {
                let p = s.pos;
                let t = s.text.clone();
                self.err(p, format!("{t:?} must be style.{t}"));
                return;
            }
        }
        match kw {
            Some(k) => {
                let Some(obj) = self.resolve(scope, &path[..k], true) else {
                    return;
                };
                self.field(obj, &path[k..], st);
            }
            None => {
                let Some(obj) = self.resolve(scope, path, true) else {
                    return;
                };
                if let Some(v) = &st.value {
                    if let Some(v) = self.subst(v) {
                        self.set_label(obj, &v);
                    }
                }
                if let Some(m) = &st.map {
                    self.map(obj, m);
                }
            }
        }
    }

    fn set_label(&mut self, obj: usize, v: &Value) {
        if obj == 0 {
            self.warn_once("a label on the root board is ignored by the native backend".into());
            return;
        }
        let o = &mut self.g.objects[obj];
        if self.weak && o.label.is_some() {
            return;
        }
        o.label = Some(v.text.clone());
        o.label_block = v.block;
    }

    /// Find or create the object at `path` under `scope`.
    fn resolve(&mut self, scope: usize, path: &[Seg], create: bool) -> Option<usize> {
        let mut cur = scope;
        for seg in path {
            if !seg.quoted && seg.text == "_" {
                if cur == 0 {
                    self.err(seg.pos, "parent \"_\" cannot be used in the root scope");
                    return None;
                }
                cur = self.g.objects[cur].parent;
                continue;
            }
            if is_reserved(seg) || is_style_keyword(seg) {
                self.err(
                    seg.pos,
                    format!("reserved keywords are prohibited in edges: {:?}", seg.text),
                );
                return None;
            }
            let found = self.g.objects[cur]
                .children
                .iter()
                .copied()
                .find(|&c| self.g.objects[c].id.eq_ignore_ascii_case(&seg.text));
            cur = match found {
                Some(c) => c,
                None if create => {
                    let mut segs: Vec<String> = Vec::new();
                    let mut i = cur;
                    while i != 0 {
                        segs.push(self.g.objects[i].id.clone());
                        i = self.g.objects[i].parent;
                    }
                    segs.reverse();
                    segs.push(seg.text.clone());
                    let level = self.g.objects[cur].level + 1;
                    let idx = self.g.objects.len();
                    self.g.objects.push(Object {
                        id: seg.text.clone(),
                        abs: key_string(&segs),
                        parent: cur,
                        shape: "rectangle".into(),
                        level,
                        ..Object::default()
                    });
                    self.g.objects[cur].children.push(idx);
                    idx
                }
                None => return None,
            };
        }
        Some(cur)
    }

    fn need_value(&mut self, kw: &Seg, st: &Stmt) -> Option<Value> {
        match &st.value {
            Some(v) => self.subst(v),
            None => {
                self.err(kw.pos, format!("{} must be a value", kw.text));
                None
            }
        }
    }

    /// Apply a keyword path (`shape`, `style.fill`, ...) to `obj`.
    fn field(&mut self, obj: usize, path: &[Seg], st: &Stmt) {
        let kw = &path[0];
        let rest = &path[1..];
        let weak = self.weak;
        match kw.text.as_str() {
            "style" => self.style_field(Target::Object(obj), rest, st),
            "label" => {
                if let (true, Some(v)) = (rest.is_empty(), &st.value) {
                    if let Some(v) = self.subst(v) {
                        self.set_label(obj, &v);
                    }
                }
                if st.map.is_some() || !rest.is_empty() {
                    self.warn_once("label positioning is ignored by the native backend".into());
                }
            }
            "shape" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                let s = v.text.to_ascii_lowercase();
                if !SHAPES.contains(&s.as_str()) {
                    return self.err(v.pos, format!("unknown shape {:?}", v.text));
                }
                if UNSUPPORTED_SHAPES.contains(&s.as_str()) {
                    self.warn_once(format!(
                        "shape {s:?} is drawn as a rectangle by the native backend"
                    ));
                }
                let o = &mut self.g.objects[obj];
                if !weak || !o.shape_set {
                    o.shape = s;
                    o.shape_set = true;
                }
            }
            "direction" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                let d = match v.text.as_str() {
                    "down" => Direction::Down,
                    "up" => Direction::Up,
                    "right" => Direction::Right,
                    "left" => Direction::Left,
                    other => {
                        return self.err(
                            v.pos,
                            format!(
                                "direction must be one of up, down, right, left, got {other:?}"
                            ),
                        );
                    }
                };
                put(weak, &mut self.g.objects[obj].direction, d);
            }
            "width" | "height" | "grid-gap" | "vertical-gap" | "horizontal-gap" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                let n = match v.text.parse::<f64>() {
                    Ok(n) if n >= 0.0 && (n > 0.0 || kw.text.contains("gap")) => n,
                    _ => {
                        return self.err(
                            v.pos,
                            format!("expected {:?} to be a non-negative number", kw.text),
                        );
                    }
                };
                let o = &mut self.g.objects[obj];
                let slot = match kw.text.as_str() {
                    "width" => &mut o.width,
                    "height" => &mut o.height,
                    "grid-gap" => &mut o.grid_gap,
                    "vertical-gap" => &mut o.vertical_gap,
                    _ => &mut o.horizontal_gap,
                };
                put(weak, slot, n);
            }
            "grid-rows" | "grid-columns" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                let n = match v.text.parse::<usize>() {
                    Ok(n) if n > 0 => n,
                    _ => {
                        return self.err(
                            v.pos,
                            format!("{} must be a positive integer: {:?}", kw.text, v.text),
                        );
                    }
                };
                let o = &mut self.g.objects[obj];
                let slot = if kw.text == "grid-rows" {
                    &mut o.grid_rows
                } else {
                    &mut o.grid_columns
                };
                put(weak, slot, n);
            }
            "icon" => {
                if let Some(v) = &st.value {
                    if let Some(v) = self.subst(v) {
                        put(weak, &mut self.g.objects[obj].icon, v.text);
                    }
                }
                if st.map.is_some() || !rest.is_empty() {
                    self.warn_once("icon positioning is ignored by the native backend".into());
                }
            }
            "tooltip" | "link" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                let o = &mut self.g.objects[obj];
                let slot = if kw.text == "tooltip" {
                    &mut o.tooltip
                } else {
                    &mut o.link
                };
                put(weak, slot, v.text);
            }
            "near" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                match Near::parse(&v.text) {
                    Some(n) if self.g.objects[obj].parent == 0 && obj != 0 => {
                        put(weak, &mut self.g.objects[obj].near, n);
                    }
                    Some(_) => self.warn_once(
                        "near constants on nested shapes are ignored by the native backend".into(),
                    ),
                    None => self.pending_near.push((obj, v)),
                }
            }
            "class" => {
                let Some(v) = self.need_value(kw, st) else {
                    return;
                };
                self.queue_classes(Target::Object(obj), &v);
            }
            other => {
                self.warn_once(format!(
                    "{other:?} is not supported by the native backend and was ignored"
                ));
            }
        }
    }

    fn style_field(&mut self, target: Target, rest: &[Seg], st: &Stmt) {
        if let Some(k) = rest.first() {
            if rest.len() > 1 {
                return self.err(rest[1].pos, "style keywords cannot have children");
            }
            match &st.value {
                Some(v) => {
                    if let Some(v) = self.subst(v) {
                        self.style_value(target, k, &v);
                    }
                }
                None => self.err(k.pos, format!("style.{} must be a value", k.text)),
            }
            return;
        }
        let Some(m) = &st.map else {
            return self.err(st.pos, "style must be a map");
        };
        for inner in &m.stmts {
            if inner.is_edge() || inner.keys[0].len() != 1 {
                self.err(inner.pos, "style map entries must be single keywords");
                continue;
            }
            match &inner.value {
                Some(v) => {
                    if let Some(v) = self.subst(v) {
                        self.style_value(target, &inner.keys[0][0], &v);
                    }
                }
                None => self.err(inner.pos, "style keywords must have a value"),
            }
        }
    }

    fn style_value(&mut self, target: Target, k: &Seg, v: &Value) {
        if !is_style_keyword(k) {
            return self.err(k.pos, format!("invalid style keyword: {:?}", k.text));
        }
        let weak = self.weak;
        let num = |lo: f64, hi: f64| -> Option<f64> {
            v.text.parse::<f64>().ok().filter(|n| *n >= lo && *n <= hi)
        };
        let boolean = || match v.text.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        let style = match target {
            Target::Object(i) => &mut self.g.objects[i].style,
            Target::Edge(i) => &mut self.g.edges[i].style,
        };
        let name = k.text.as_str();
        let text = v.text.clone();
        let bad: Option<String> = match name {
            "fill" => {
                put(weak, &mut style.fill, text);
                None
            }
            "stroke" => {
                put(weak, &mut style.stroke, text);
                None
            }
            "font-color" => {
                put(weak, &mut style.font_color, text);
                None
            }
            "font" => {
                put(weak, &mut style.font, text);
                None
            }
            "text-transform" => match text.as_str() {
                "uppercase" | "lowercase" | "title" | "none" => {
                    put(weak, &mut style.text_transform, text);
                    None
                }
                _ => Some("text-transform must be one of uppercase, lowercase, title, none".into()),
            },
            "opacity" => match num(0.0, 1.0) {
                Some(n) => {
                    put(weak, &mut style.opacity, n);
                    None
                }
                None => Some("expected \"opacity\" to be a number between 0.0 and 1.0".into()),
            },
            "stroke-width" => match num(0.0, 15.0) {
                Some(n) => {
                    put(weak, &mut style.stroke_width, n);
                    None
                }
                None => Some("expected \"stroke-width\" to be a number between 0 and 15".into()),
            },
            "stroke-dash" => match num(0.0, 10.0) {
                Some(n) => {
                    put(weak, &mut style.stroke_dash, n);
                    None
                }
                None => Some("expected \"stroke-dash\" to be a number between 0 and 10".into()),
            },
            "border-radius" => match num(0.0, f64::MAX) {
                Some(n) => {
                    put(weak, &mut style.border_radius, n);
                    None
                }
                None => {
                    Some("expected \"border-radius\" to be a number greater or equal to 0".into())
                }
            },
            "font-size" => match num(8.0, 100.0) {
                Some(n) => {
                    put(weak, &mut style.font_size, n);
                    None
                }
                None => Some("expected \"font-size\" to be a number between 8 and 100".into()),
            },
            "bold" | "italic" | "underline" | "shadow" | "multiple" | "double-border"
            | "animated" | "3d" | "filled" => match boolean() {
                Some(b) => {
                    let slot = match name {
                        "bold" => &mut style.bold,
                        "italic" => &mut style.italic,
                        "underline" => &mut style.underline,
                        "shadow" => &mut style.shadow,
                        "multiple" => &mut style.multiple,
                        "double-border" => &mut style.double_border,
                        "animated" => &mut style.animated,
                        "3d" => &mut style.three_d,
                        _ => &mut style.filled,
                    };
                    put(weak, slot, b);
                    None
                }
                None => Some(format!("expected {name:?} to be true or false")),
            },
            _ => {
                let msg =
                    format!("style.{name} is not supported by the native backend and was ignored");
                self.warn_once(msg);
                None
            }
        };
        if let Some(msg) = bad {
            self.err(v.pos, msg);
        } else if name == "animated" && v.text == "true" {
            self.warn_once(
                "style.animated is drawn without animation by the native backend".into(),
            );
        }
    }

    fn edge_stmt(&mut self, scope: usize, st: &Stmt) {
        let mut ends = Vec::new();
        for k in &st.keys {
            match self.resolve(scope, k, true) {
                Some(o) => ends.push(o),
                None => return,
            }
        }
        if ends.contains(&0) {
            return self.err(st.pos, "connections cannot target the root board");
        }
        let label = match &st.value {
            Some(v) => match self.subst(v) {
                Some(v) => Some(v.text),
                None => return,
            },
            None => None,
        };
        let scope_key = self.g.objects[scope].abs.clone();
        let rel = |c: &Compiler, o: usize| -> String {
            let abs = &c.g.objects[o].abs;
            if scope_key.is_empty() {
                abs.clone()
            } else {
                abs.strip_prefix(&format!("{scope_key}."))
                    .unwrap_or(abs)
                    .to_string()
            }
        };
        for (i, op) in st.ops.iter().enumerate() {
            let (a, b) = (ends[i], ends[i + 1]);
            let base = format!("({} {} {})", rel(self, a), op.as_str(), rel(self, b));
            let full_base = if scope_key.is_empty() {
                base
            } else {
                format!("{scope_key}.{base}")
            };
            let n = self.edge_counts.entry(full_base.clone()).or_insert(0);
            let key = format!("{full_base}[{n}]");
            *n += 1;
            let head = || {
                Some(Arrowhead {
                    drawn: true,
                    ..Arrowhead::default()
                })
            };
            let (src_head, dst_head) = match op {
                EdgeOp::Forward => (None, head()),
                EdgeOp::Backward => (head(), None),
                EdgeOp::Both => (head(), head()),
                EdgeOp::Undirected => (None, None),
            };
            self.g.edges.push(Edge {
                src: a,
                dst: b,
                src_head,
                dst_head,
                label: label.clone(),
                style: Style::default(),
                key,
                tooltip: None,
                link: None,
                classes: Vec::new(),
            });
            let ei = self.g.edges.len() - 1;
            if let Some(m) = &st.map {
                self.edge_map(ei, m);
            }
        }
    }

    fn edge_map(&mut self, ei: usize, m: &Map) {
        let weak = self.weak;
        for inner in &m.stmts {
            if inner.is_edge() {
                self.err(
                    inner.pos,
                    "connections cannot be declared inside a connection",
                );
                continue;
            }
            let path: &KeyPath = &inner.keys[0];
            let head = &path[0];
            let value = match &inner.value {
                Some(v) => match self.subst(v) {
                    Some(v) => Some(v),
                    None => continue,
                },
                None => None,
            };
            match (head.quoted, head.text.as_str()) {
                (false, "label") => {
                    if path.len() > 1 || inner.map.is_some() {
                        self.warn_once("label positioning is ignored by the native backend".into());
                    }
                    if let (1, Some(v)) = (path.len(), value) {
                        put(weak, &mut self.g.edges[ei].label, v.text);
                    }
                }
                (false, "style") => self.style_field(Target::Edge(ei), &path[1..], inner),
                (false, "source-arrowhead" | "target-arrowhead") => {
                    let src = head.text == "source-arrowhead";
                    self.arrowhead(ei, src, &path[1..], value.as_ref(), inner.map.as_ref());
                }
                (false, "class") => {
                    if let Some(v) = value {
                        self.queue_classes(Target::Edge(ei), &v);
                    }
                }
                (false, "tooltip") => {
                    if let Some(v) = value {
                        put(weak, &mut self.g.edges[ei].tooltip, v.text);
                    }
                }
                (false, "link") => {
                    if let Some(v) = value {
                        put(weak, &mut self.g.edges[ei].link, v.text);
                    }
                }
                _ => {
                    self.warn_once(format!(
                        "{:?} on a connection is ignored by the native backend",
                        head.text
                    ));
                }
            }
        }
    }

    /// `target-arrowhead: label`, `target-arrowhead.shape: diamond`,
    /// `target-arrowhead: { shape: circle; style.filled: true; label: 1 }`.
    fn arrowhead(
        &mut self,
        ei: usize,
        src: bool,
        rest: &[Seg],
        value: Option<&Value>,
        map: Option<&Map>,
    ) {
        let mut entries: Vec<(Vec<String>, Value)> = Vec::new();
        match (rest.is_empty(), value) {
            (true, Some(v)) => entries.push((vec!["label".into()], v.clone())),
            (false, Some(v)) => {
                entries.push((rest.iter().map(|s| s.text.clone()).collect(), v.clone()))
            }
            _ => {}
        }
        if let Some(m) = map {
            for st in &m.stmts {
                if st.is_edge() {
                    continue;
                }
                let mut k: Vec<String> = rest.iter().map(|s| s.text.clone()).collect();
                k.extend(st.keys[0].iter().map(|s| s.text.clone()));
                if let Some(v) = &st.value {
                    if let Some(v) = self.subst(v) {
                        entries.push((k.clone(), v));
                    }
                }
                if let Some(inner) = &st.map {
                    for s2 in &inner.stmts {
                        if let (false, Some(v)) = (s2.is_edge(), &s2.value) {
                            let mut k2 = k.clone();
                            k2.extend(s2.keys[0].iter().map(|s| s.text.clone()));
                            if let Some(v) = self.subst(v) {
                                entries.push((k2, v));
                            }
                        }
                    }
                }
            }
        }
        let weak = self.weak;
        let edge = &mut self.g.edges[ei];
        let slot = if src {
            &mut edge.src_head
        } else {
            &mut edge.dst_head
        };
        let mut h = slot.clone().unwrap_or_default();
        let mut errors = Vec::new();
        for (k, v) in entries {
            let k: Vec<&str> = k.iter().map(String::as_str).collect();
            match k.as_slice() {
                ["label"] => put(weak, &mut h.label, v.text),
                ["shape"] => match ArrowShape::parse(&v.text) {
                    Some(s) => {
                        if !weak || slot.is_none() || h.shape == ArrowShape::Triangle {
                            h.shape = s;
                        }
                    }
                    None => errors.push((v.pos, format!("unknown shape {:?}", v.text))),
                },
                ["style", "filled"] => match v.text.as_str() {
                    "true" => put(weak, &mut h.filled, true),
                    "false" => put(weak, &mut h.filled, false),
                    _ => errors.push((v.pos, "expected \"filled\" to be true or false".into())),
                },
                _ => {}
            }
        }
        *slot = Some(h);
        for (p, m) in errors {
            self.err(p, m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::parse;
    use super::*;

    fn g(src: &str) -> Graph {
        compile(&parse(src).unwrap()).unwrap()
    }

    fn obj<'a>(g: &'a Graph, abs: &str) -> &'a Object {
        g.objects.iter().find(|o| o.abs == abs).unwrap()
    }

    #[test]
    fn objects_and_edges() {
        let g = g("a: Alpha\ngrp: {\n  c\n  c -> d\n}\na -> grp.c\na -> grp.c\n");
        let abs: Vec<_> = g.objects.iter().skip(1).map(|o| o.abs.as_str()).collect();
        assert_eq!(abs, ["a", "grp", "grp.c", "grp.d"]);
        let keys: Vec<_> = g.edges.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            ["grp.(c -> d)[0]", "(a -> grp.c)[0]", "(a -> grp.c)[1]"]
        );
        assert_eq!(g.objects[1].label.as_deref(), Some("Alpha"));
    }

    #[test]
    fn errors() {
        let e = compile(&parse("x: {shape: nosuch}\ny.style.opacity: 7\n").unwrap()).unwrap_err();
        let msgs: Vec<_> = e.iter().map(|d| d.to_string()).collect();
        assert_eq!(
            msgs,
            [
                "1:12: unknown shape \"nosuch\"",
                "2:18: expected \"opacity\" to be a number between 0.0 and 1.0"
            ]
        );
    }

    #[test]
    fn styles_and_shapes() {
        let g = g(
            "a.shape: circle\na.style: {fill: red; stroke-width: 3}\nb -> a: {style.stroke: blue}\n",
        );
        let a = &g.objects[1];
        assert_eq!(a.shape, "circle");
        assert_eq!(a.style.fill.as_deref(), Some("red"));
        assert_eq!(a.style.stroke_width, Some(3.0));
        assert_eq!(g.edges[0].style.stroke.as_deref(), Some("blue"));
    }

    #[test]
    fn vars_substitute_in_scope() {
        let g = g(concat!(
            "vars: {color: red; srv: {name: API}}\n",
            "a: ${srv.name} server {style.fill: ${color}}\n",
            "b: {\n  vars: {color: blue}\n  c.style.fill: ${color}\n}\n",
            "d: '${color}'\n",
        ));
        assert_eq!(obj(&g, "a").label.as_deref(), Some("API server"));
        assert_eq!(obj(&g, "a").style.fill.as_deref(), Some("red"));
        assert_eq!(obj(&g, "b.c").style.fill.as_deref(), Some("blue"));
        assert_eq!(obj(&g, "d").label.as_deref(), Some("${color}"));
        let e = compile(&parse("a: ${nope}\n").unwrap()).unwrap_err();
        assert_eq!(e[0].message, "could not resolve variable \"nope\"");
    }

    #[test]
    fn d2_config_is_read() {
        let g = g("vars: {d2-config: {theme-id: 4; pad: 10}}\na\n");
        assert_eq!(g.config.theme_id, Some(4));
        assert_eq!(g.config.pad, Some(10.0));
    }

    #[test]
    fn classes_apply_and_explicit_wins() {
        let g = g(concat!(
            "classes: {\n  db: {shape: cylinder; style.fill: yellow}\n  big: {style.font-size: 30; style.fill: pink}\n}\n",
            "a.class: db\n",
            "b: {class: [db; big]; style.fill: green}\n",
            "c.class: [big; db]\n",
            "a -> b: {class: big}\n",
        ));
        assert_eq!(obj(&g, "a").shape, "cylinder");
        assert_eq!(obj(&g, "a").style.fill.as_deref(), Some("yellow"));
        assert_eq!(obj(&g, "b").style.fill.as_deref(), Some("green"));
        assert_eq!(obj(&g, "b").style.font_size, Some(30.0));
        // The last class wins.
        assert_eq!(obj(&g, "c").style.fill.as_deref(), Some("yellow"));
        assert_eq!(g.edges[0].style.font_size, Some(30.0));
    }

    #[test]
    fn near_icon_tooltip_link_grid_arrowheads() {
        let g = g(concat!(
            "title: Hi {near: top-center}\n",
            "a: {icon: https://x/y.svg; tooltip: tip; link: https://l}\n",
            "g: {grid-columns: 2; grid-gap: 10; p; q; r}\n",
            "a -> g: {target-arrowhead: {shape: diamond; style.filled: true; label: 1..*}}\n",
            "a -- g: {source-arrowhead.shape: circle}\n",
        ));
        assert_eq!(obj(&g, "title").near, Some(Near::TopCenter));
        let a = obj(&g, "a");
        assert_eq!(a.icon.as_deref(), Some("https://x/y.svg"));
        assert_eq!(a.tooltip.as_deref(), Some("tip"));
        assert_eq!(a.link.as_deref(), Some("https://l"));
        assert_eq!(obj(&g, "g").grid_columns, Some(2));
        let h = g.edges[0].dst_head.as_ref().unwrap();
        assert_eq!(
            (h.shape, h.filled, h.label.as_deref()),
            (ArrowShape::Diamond, Some(true), Some("1..*"))
        );
        let s = g.edges[1].src_head.as_ref().unwrap();
        assert_eq!((s.shape, s.drawn), (ArrowShape::Circle, false));
        assert!(g.edges[1].dst_head.is_none());
    }

    #[test]
    fn near_errors_match_d2() {
        let e = compile(&parse("a: {near: nowhere}\n").unwrap()).unwrap_err();
        assert_eq!(
            e[0].to_string(),
            "1:11: near key \"nowhere\" must be the absolute path to a shape or one of the following constants: top-left, top-center, top-right, center-left, center-right, bottom-left, bottom-center, bottom-right"
        );
        let g = g("a\nb.near: a\n");
        assert!(g.warnings.iter().any(|w| w.contains("near an object")));
    }
}
