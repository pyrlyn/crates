// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Layered layout in the spirit of D2's dagre layout (d2layouts/d2dagrelayout)
// and shape sizing from d2graph/lib/shape, Copyright 2022 Terrastruct, Inc.
// Rewritten in Rust as a self-contained Sugiyama-style layout.

//! Layered (Sugiyama-style) layout with nested containers.
//!
//! Each container is laid out on its own: its children are ranked by
//! longest path (after breaking cycles), ordered by barycenter sweeps with
//! dummy nodes for edges that span ranks, and placed with a balanced
//! two-pass coordinate assignment. A container's size is then the box around
//! its laid-out children, so nesting composes bottom-up.

use super::graph::{Direction, Graph, Near};
use super::text::{measure, measure_mono};

/// Font size of shape labels.
pub const FONT_SIZE: f64 = 16.0;
const NODE_PAD_X: f64 = 22.0;
const NODE_PAD_Y: f64 = 22.0;
const CONTAINER_PAD: f64 = 30.0;
const NODE_SEP: f64 = 60.0;
const DUMMY_SEP: f64 = 20.0;
const RANK_SEP: f64 = 60.0;
const EDGE_LABEL_FONT: f64 = 16.0;
const PARALLEL_GAP: f64 = 24.0;
/// Icon size inside a shape.
pub const ICON_SIZE: f64 = 32.0;
/// Icon size next to a container's label.
pub const CONTAINER_ICON: f64 = 24.0;
const NEAR_GAP: f64 = 30.0;
const GRID_GAP: f64 = 40.0;

/// Axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl Rect {
    /// Center point.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Where a connection goes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Route {
    /// Polyline from source boundary to destination boundary.
    pub points: Vec<(f64, f64)>,
    /// Label box, if the connection has a label.
    pub label: Option<Rect>,
}

/// Result of a layout.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Absolute box per object (index 0, the root, spans everything).
    pub boxes: Vec<Rect>,
    /// Label font size per object.
    pub font_sizes: Vec<f64>,
    /// Route per edge.
    pub routes: Vec<Route>,
    /// Bounding box of everything drawn.
    pub bounds: Rect,
}

/// Font size used for an object's label.
pub fn font_size(g: &Graph, i: usize) -> f64 {
    let o = &g.objects[i];
    if let Some(s) = o.style.font_size {
        return s;
    }
    if o.is_container() {
        match o.level {
            1 => 28.0,
            2 => 24.0,
            3 => 20.0,
            _ => FONT_SIZE,
        }
    } else {
        FONT_SIZE
    }
}

fn is_bold(g: &Graph, i: usize) -> bool {
    let o = &g.objects[i];
    o.style
        .bold
        .unwrap_or(o.is_container() || o.shape != "text")
}

/// Size of a leaf shape from its label.
fn leaf_size(g: &Graph, i: usize) -> (f64, f64) {
    let o = &g.objects[i];
    let fs = font_size(g, i);
    let mono = o.shape == "code" || o.style.font.as_deref() == Some("mono");
    let (tw, th) = if mono {
        measure_mono(o.label_text(), fs)
    } else {
        measure(o.label_text(), fs, is_bold(g, i))
    };
    let (mut w, mut h) = match o.shape.as_str() {
        "image" => (tw.max(128.0), 128.0 + th + 8.0),
        "text" => (tw, th),
        "square" => {
            let s = (tw + 2.0 * NODE_PAD_X).max(th + 2.0 * NODE_PAD_Y);
            (s, s)
        }
        "circle" => {
            let d = (tw + 30.0).hypot(th + 30.0);
            (d, d)
        }
        "oval" => (
            (tw + 2.0 * NODE_PAD_X) * 1.25,
            (th + 2.0 * NODE_PAD_Y) * 1.15,
        ),
        "diamond" => (tw * 1.8 + 40.0, th * 1.8 + 40.0),
        "cloud" => (tw * 1.6 + 40.0, th * 1.6 + 40.0),
        "hexagon" | "parallelogram" | "step" => (tw + 80.0, th + 2.0 * NODE_PAD_Y),
        "queue" => (tw + 70.0, th + 2.0 * NODE_PAD_Y),
        "cylinder" | "stored_data" => (tw + 2.0 * NODE_PAD_X + 10.0, th + 2.0 * NODE_PAD_Y + 24.0),
        "document" => (tw + 2.0 * NODE_PAD_X, th + 2.0 * NODE_PAD_Y + 12.0),
        "package" | "page" => (tw + 2.0 * NODE_PAD_X + 10.0, th + 2.0 * NODE_PAD_Y + 10.0),
        "person" | "c4-person" => ((tw + 20.0).max(70.0), th + 100.0),
        "callout" => (tw + 2.0 * NODE_PAD_X, th + 2.0 * NODE_PAD_Y + 20.0),
        _ => (tw + 2.0 * NODE_PAD_X, th + 2.0 * NODE_PAD_Y),
    };
    if o.icon.is_some() && o.shape != "image" && o.shape != "text" {
        h += ICON_SIZE + 8.0;
        w = w.max(ICON_SIZE + 2.0 * NODE_PAD_X);
    }
    (
        o.width.unwrap_or(w).round().max(5.0),
        o.height.unwrap_or(h).round().max(5.0),
    )
}

/// Space above a container's children taken by its label.
fn container_top(g: &Graph, i: usize) -> f64 {
    let (_, th) = measure(g.objects[i].label_text(), font_size(g, i), true);
    th + CONTAINER_PAD
}

struct Ctx<'g> {
    g: &'g Graph,
    /// Size per object.
    size: Vec<(f64, f64)>,
    /// Position of each object relative to its parent's content origin.
    rel: Vec<(f64, f64)>,
    /// Bend points per edge, relative to the content origin of `lca[e]`.
    bends: Vec<Vec<(f64, f64)>>,
    /// Container whose layout owns each edge.
    lca: Vec<usize>,
    /// The two children of `lca[e]` the edge connects.
    ends: Vec<Option<(usize, usize)>>,
}

/// Lay out a compiled graph.
pub fn layout(g: &Graph) -> Layout {
    let n = g.objects.len();
    let mut ctx = Ctx {
        g,
        size: vec![(0.0, 0.0); n],
        rel: vec![(0.0, 0.0); n],
        bends: vec![Vec::new(); g.edges.len()],
        lca: vec![0; g.edges.len()],
        ends: vec![None; g.edges.len()],
    };
    for (ei, e) in g.edges.iter().enumerate() {
        let (lca, ends) = lift(g, e.src, e.dst);
        ctx.lca[ei] = lca;
        ctx.ends[ei] = ends;
    }
    let root_dir = g.objects[0].direction.unwrap_or_default();
    let (w, h) = ctx.size_of(0, root_dir);
    ctx.size[0] = (w, h);
    ctx.place_near(w, h);

    // Absolute boxes, top-down.
    let mut boxes = vec![Rect::default(); n];
    boxes[0] = Rect {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    let mut origin = vec![(0.0, 0.0); n];
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        let o = &g.objects[i];
        if i != 0 {
            let (px, py) = origin[o.parent];
            let (rx, ry) = ctx.rel[i];
            let (w, h) = ctx.size[i];
            boxes[i] = Rect {
                x: px + rx,
                y: py + ry,
                w,
                h,
            };
        }
        origin[i] = if i == 0 {
            (0.0, 0.0)
        } else {
            (boxes[i].x + CONTAINER_PAD, boxes[i].y + container_top(g, i))
        };
        stack.extend(o.children.iter().copied());
    }

    let mut routes = Vec::with_capacity(g.edges.len());
    let mut groups: std::collections::HashMap<(usize, usize), usize> = Default::default();
    let mut group_size: std::collections::HashMap<(usize, usize), usize> = Default::default();
    for e in &g.edges {
        *group_size.entry(pair(e.src, e.dst)).or_insert(0) += 1;
    }
    for (ei, e) in g.edges.iter().enumerate() {
        let (ox, oy) = origin[ctx.lca[ei]];
        let sb = boxes[e.src];
        let db = boxes[e.dst];
        let mut pts: Vec<(f64, f64)> = Vec::new();
        if e.src == e.dst {
            let r = sb;
            let (_, cy) = r.center();
            let k = groups.entry(pair(e.src, e.dst)).or_insert(0);
            let off = 20.0 + *k as f64 * 15.0;
            *k += 1;
            pts.push((r.x + r.w, cy - r.h / 4.0));
            pts.push((r.x + r.w + off + 20.0, cy - r.h / 4.0));
            pts.push((r.x + r.w + off + 20.0, cy + r.h / 4.0));
            pts.push((r.x + r.w, cy + r.h / 4.0));
        } else {
            let mut mid: Vec<(f64, f64)> = ctx.bends[ei]
                .iter()
                .map(|&(x, y)| (x + ox, y + oy))
                .collect();
            let key = pair(e.src, e.dst);
            let count = group_size[&key];
            let k = groups.entry(key).or_insert(0);
            let idx = *k;
            *k += 1;
            let s = sb.center();
            let d = db.center();
            if count > 1 && mid.is_empty() {
                let off = (idx as f64 - (count as f64 - 1.0) / 2.0) * PARALLEL_GAP;
                let (dx, dy) = (d.0 - s.0, d.1 - s.1);
                let len = dx.hypot(dy).max(1e-6);
                let (nx, ny) = (-dy / len, dx / len);
                // Keep the offset direction stable regardless of edge direction.
                let sign = if e.src < e.dst { 1.0 } else { -1.0 };
                mid.push((
                    (s.0 + d.0) / 2.0 + nx * off * sign,
                    (s.1 + d.1) / 2.0 + ny * off * sign,
                ));
            }
            let first_target = mid.first().copied().unwrap_or(d);
            let last_source = mid.last().copied().unwrap_or(s);
            let contains = g.contains(e.src, e.dst) || g.contains(e.dst, e.src);
            let start = if contains && g.contains(e.src, e.dst) {
                s
            } else {
                clip(&g.objects[e.src].shape, sb, first_target)
            };
            let end = if contains && g.contains(e.dst, e.src) {
                d
            } else {
                clip(&g.objects[e.dst].shape, db, last_source)
            };
            pts.push(start);
            pts.extend(mid);
            pts.push(end);
        }
        let label = e.label.as_ref().map(|t| {
            let (tw, th) = measure(t, EDGE_LABEL_FONT, false);
            let (mx, my) = midpoint(&pts);
            Rect {
                x: mx - tw / 2.0 - 4.0,
                y: my - th / 2.0,
                w: tw + 8.0,
                h: th,
            }
        });
        routes.push(Route { points: pts, label });
    }

    let mut bounds = boxes[0];
    let mut grow = |x: f64, y: f64| {
        let x2 = (bounds.x + bounds.w).max(x);
        let y2 = (bounds.y + bounds.h).max(y);
        bounds.x = bounds.x.min(x);
        bounds.y = bounds.y.min(y);
        bounds.w = x2 - bounds.x;
        bounds.h = y2 - bounds.y;
    };
    for b in boxes.iter().skip(1) {
        grow(b.x, b.y);
        grow(b.x + b.w, b.y + b.h);
    }
    for r in &routes {
        for &(x, y) in &r.points {
            grow(x, y);
        }
        if let Some(l) = r.label {
            grow(l.x, l.y);
            grow(l.x + l.w, l.y + l.h);
        }
    }
    let font_sizes = (0..n).map(|i| font_size(g, i)).collect();
    Layout {
        boxes,
        font_sizes,
        routes,
        bounds,
    }
}

fn pair(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// Lowest container that strictly holds both ends, and the children of it
/// that hold each end (`None` when one end contains the other).
fn lift(g: &Graph, src: usize, dst: usize) -> (usize, Option<(usize, usize)>) {
    let chain = |i: usize| {
        let mut v = vec![i];
        v.extend(g.ancestors(i));
        v.reverse();
        v // root .. i
    };
    let a = chain(src);
    let b = chain(dst);
    let mut k = 0;
    while k < a.len() && k < b.len() && a[k] == b[k] {
        k += 1;
    }
    // a[k-1] is the deepest common node.
    let common = a[k - 1];
    if common == src || common == dst {
        if src == dst {
            let parent = g.objects[src].parent;
            return (parent, Some((src, src)));
        }
        let lca = if common == 0 {
            0
        } else {
            g.objects[common].parent
        };
        return (lca, None);
    }
    (common, Some((a[k], b[k])))
}

impl Ctx<'_> {
    /// Put root shapes with a `near` constant around the `w` x `h`
    /// diagram; several shapes at one constant stack outwards.
    fn place_near(&mut self, w: f64, h: f64) {
        let mut stack: Vec<(Near, f64)> = Vec::new();
        for &c in &self.g.objects[0].children {
            let Some(near) = self.g.objects[c].near else {
                continue;
            };
            let (ow, oh) = self.size[c];
            let off = stack
                .iter()
                .filter(|(n, _)| *n == near)
                .map(|(_, d)| d)
                .sum::<f64>();
            let vertical = !matches!(near, Near::CenterLeft | Near::CenterRight);
            stack.push((
                near,
                if vertical {
                    oh + NEAR_GAP
                } else {
                    ow + NEAR_GAP
                },
            ));
            let above = -oh - NEAR_GAP - off;
            let below = h + NEAR_GAP + off;
            self.rel[c] = match near {
                Near::TopLeft => (0.0, above),
                Near::TopCenter => ((w - ow) / 2.0, above),
                Near::TopRight => (w - ow, above),
                Near::CenterLeft => (-ow - NEAR_GAP - off, (h - oh) / 2.0),
                Near::CenterRight => (w + NEAR_GAP + off, (h - oh) / 2.0),
                Near::BottomLeft => (0.0, below),
                Near::BottomCenter => ((w - ow) / 2.0, below),
                Near::BottomRight => (w - ow, below),
            };
        }
    }

    /// Size of object `i`, laying out its children first.
    fn size_of(&mut self, i: usize, inherited: Direction) -> (f64, f64) {
        let g = self.g;
        let o = &g.objects[i];
        if o.children.is_empty() {
            return leaf_size(self.g, i);
        }
        let dir = o.direction.unwrap_or(inherited);
        for &c in &o.children {
            let s = self.size_of(c, dir);
            self.size[c] = s;
        }
        // Root shapes with a `near` constant are placed around the diagram
        // afterwards, not in the layered layout.
        let children: Vec<usize> = o
            .children
            .iter()
            .copied()
            .filter(|&c| i != 0 || self.g.objects[c].near.is_none())
            .collect();
        let (cw, ch) = if o.is_grid() {
            self.layout_grid(i, &children)
        } else {
            self.layout_children(i, &children, dir)
        };
        if i == 0 {
            return (cw, ch);
        }
        let (mut lw, _) = measure(o.label_text(), font_size(self.g, i), true);
        if o.icon.is_some() {
            lw += CONTAINER_ICON + 12.0;
        }
        let w = (cw + 2.0 * CONTAINER_PAD).max(lw + 2.0 * CONTAINER_PAD);
        let h = ch + container_top(self.g, i) + CONTAINER_PAD;
        (
            o.width.unwrap_or(w).max(w).round(),
            o.height.unwrap_or(h).max(h).round(),
        )
    }

    /// Grid layout (`grid-rows` / `grid-columns`); returns the content size.
    fn layout_grid(&mut self, parent: usize, children: &[usize]) -> (f64, f64) {
        let o = &self.g.objects[parent];
        let n = children.len().max(1);
        let (rows, cols, row_major) = match (o.grid_rows, o.grid_columns) {
            (Some(r), Some(c)) => (r.max(n.div_ceil(c)), c, true),
            (None, Some(c)) => (n.div_ceil(c), c, true),
            (Some(r), None) => (r, n.div_ceil(r), false),
            (None, None) => (1, n, true),
        };
        let gap = o.grid_gap.unwrap_or(GRID_GAP);
        let vgap = o.vertical_gap.unwrap_or(gap);
        let hgap = o.horizontal_gap.unwrap_or(gap);
        let cell = |k: usize| {
            if row_major {
                (k / cols, k % cols)
            } else {
                (k % rows, k / rows)
            }
        };
        let mut col_w = vec![0.0f64; cols];
        let mut row_h = vec![0.0f64; rows];
        for (k, &c) in children.iter().enumerate() {
            let (r, cl) = cell(k);
            let (w, h) = self.size[c];
            col_w[cl] = col_w[cl].max(w);
            row_h[r] = row_h[r].max(h);
        }
        let mut xs = vec![0.0; cols];
        for c in 1..cols {
            xs[c] = xs[c - 1] + col_w[c - 1] + hgap;
        }
        let mut ys = vec![0.0; rows];
        for r in 1..rows {
            ys[r] = ys[r - 1] + row_h[r - 1] + vgap;
        }
        for (k, &c) in children.iter().enumerate() {
            let (r, cl) = cell(k);
            // Cells stretch their shape, as in D2.
            self.size[c] = (col_w[cl], row_h[r]);
            self.rel[c] = (xs[cl], ys[r]);
        }
        let w = xs.last().copied().unwrap_or(0.0) + col_w.last().copied().unwrap_or(0.0);
        let h = ys.last().copied().unwrap_or(0.0) + row_h.last().copied().unwrap_or(0.0);
        (w, h)
    }

    /// Place `children` of `parent`; returns the content size.
    fn layout_children(&mut self, parent: usize, children: &[usize], dir: Direction) -> (f64, f64) {
        let n = children.len();
        let local = |o: usize| children.iter().position(|&c| c == o);
        let horizontal = matches!(dir, Direction::Right | Direction::Left);
        // Node sizes in layout space (x across ranks, y along ranks).
        let mut w: Vec<f64> = Vec::new();
        let mut h: Vec<f64> = Vec::new();
        for &c in children {
            let (cw, ch) = self.size[c];
            if horizontal {
                w.push(ch);
                h.push(cw);
            } else {
                w.push(cw);
                h.push(ch);
            }
        }
        // Edges owned by this container.
        let mut edges: Vec<(usize, usize, usize, f64)> = Vec::new(); // (u, v, edge idx, label extent)
        for (ei, e) in self.g.edges.iter().enumerate() {
            if self.lca[ei] != parent {
                continue;
            }
            if let Some((a, b)) = self.ends[ei] {
                if let (Some(u), Some(v)) = (local(a), local(b)) {
                    if u != v {
                        let lab = e
                            .label
                            .as_ref()
                            .map(|t| {
                                let (tw, th) = measure(t, EDGE_LABEL_FONT, false);
                                if horizontal {
                                    tw + 20.0
                                } else {
                                    th + 10.0
                                }
                            })
                            .unwrap_or(0.0);
                        edges.push((u, v, ei, lab));
                    }
                }
            }
        }

        // 1. Break cycles: DFS in declaration order, reverse back edges.
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (k, &(u, _, _, _)) in edges.iter().enumerate() {
            adj[u].push(k);
        }
        let mut state = vec![0u8; n];
        let mut reversed = vec![false; edges.len()];
        for s in 0..n {
            if state[s] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
            state[s] = 1;
            while let Some(top) = stack.last_mut() {
                let u = top.0;
                if top.1 < adj[u].len() {
                    let k = adj[u][top.1];
                    top.1 += 1;
                    let v = edges[k].1;
                    match state[v] {
                        0 => {
                            state[v] = 1;
                            stack.push((v, 0));
                        }
                        1 => reversed[k] = true,
                        _ => {}
                    }
                } else {
                    state[u] = 2;
                    stack.pop();
                }
            }
        }
        let dag: Vec<(usize, usize, usize, f64)> = edges
            .iter()
            .enumerate()
            .map(|(k, &(u, v, ei, l))| {
                if reversed[k] {
                    (v, u, ei, l)
                } else {
                    (u, v, ei, l)
                }
            })
            .collect();

        // 2. Rank by longest path, then pull sources down next to their
        //    successors so they do not all sit on rank 0.
        let mut rank = vec![0usize; n];
        let mut indeg = vec![0usize; n];
        for &(_, v, _, _) in &dag {
            indeg[v] += 1;
        }
        let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut qi = 0;
        while qi < queue.len() {
            let u = queue[qi];
            qi += 1;
            for &(a, b, _, _) in &dag {
                if a == u {
                    rank[b] = rank[b].max(rank[u] + 1);
                    indeg[b] -= 1;
                    if indeg[b] == 0 {
                        queue.push(b);
                    }
                }
            }
        }
        for _ in 0..2 {
            for u in 0..n {
                let has_in = dag.iter().any(|&(_, b, _, _)| b == u);
                if has_in {
                    continue;
                }
                if let Some(m) = dag
                    .iter()
                    .filter(|&&(a, _, _, _)| a == u)
                    .map(|&(_, b, _, _)| rank[b])
                    .min()
                {
                    if m >= 1 {
                        rank[u] = m - 1;
                    }
                }
            }
        }
        let nranks = rank.iter().copied().max().map_or(1, |m| m + 1);

        // 3. Dummy nodes for long edges.
        let mut node_rank = rank.clone();
        let mut node_w = w.clone();
        let mut node_h = h.clone();
        let mut is_dummy = vec![false; n];
        let mut segs: Vec<(usize, usize)> = Vec::new();
        let mut chains: Vec<(usize, Vec<usize>, bool)> = Vec::new(); // (edge idx, dummies, reversed)
        for (k, &(u, v, ei, _)) in dag.iter().enumerate() {
            let mut prev = u;
            let mut dummies = Vec::new();
            for r in rank[u] + 1..rank[v] {
                let d = node_rank.len();
                node_rank.push(r);
                node_w.push(10.0);
                node_h.push(0.0);
                is_dummy.push(true);
                segs.push((prev, d));
                dummies.push(d);
                prev = d;
            }
            segs.push((prev, v));
            chains.push((ei, dummies, reversed[k]));
        }
        let total = node_rank.len();

        // 4. Order within ranks: initial by declaration, then barycenter sweeps.
        let mut layers: Vec<Vec<usize>> = vec![Vec::new(); nranks];
        for v in 0..total {
            layers[node_rank[v]].push(v);
        }
        let preds = |v: usize, segs: &[(usize, usize)]| -> Vec<usize> {
            segs.iter().filter(|s| s.1 == v).map(|s| s.0).collect()
        };
        let succs = |v: usize, segs: &[(usize, usize)]| -> Vec<usize> {
            segs.iter().filter(|s| s.0 == v).map(|s| s.1).collect()
        };
        let mut best = layers.clone();
        let mut best_cross = crossings(&layers, &segs, total);
        for iter in 0..16 {
            let down = iter % 2 == 0;
            let mut pos = vec![0.0f64; total];
            for layer in &layers {
                for (i, &v) in layer.iter().enumerate() {
                    pos[v] = i as f64;
                }
            }
            let ranks: Vec<usize> = if down {
                (1..nranks).collect()
            } else {
                (0..nranks.saturating_sub(1)).rev().collect()
            };
            for r in ranks {
                let mut keyed: Vec<(f64, usize)> = layers[r]
                    .iter()
                    .map(|&v| {
                        let nb = if down {
                            preds(v, &segs)
                        } else {
                            succs(v, &segs)
                        };
                        let bc = if nb.is_empty() {
                            pos[v]
                        } else {
                            nb.iter().map(|&u| pos[u]).sum::<f64>() / nb.len() as f64
                        };
                        (bc, v)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                layers[r] = keyed.into_iter().map(|(_, v)| v).collect();
                for (i, &v) in layers[r].iter().enumerate() {
                    pos[v] = i as f64;
                }
            }
            let c = crossings(&layers, &segs, total);
            if c < best_cross {
                best_cross = c;
                best = layers.clone();
            }
        }
        let layers = best;

        // 5. Coordinates. Ranks stack along y; nodes spread along x.
        let mut gap_after = vec![RANK_SEP; nranks];
        for &(u, v, _, lab) in &dag {
            if lab > 0.0 {
                let r = rank[u].min(rank[v]);
                gap_after[r] = gap_after[r].max(RANK_SEP + lab + 20.0);
            }
        }
        let rank_h: Vec<f64> = layers
            .iter()
            .map(|l| l.iter().map(|&v| node_h[v]).fold(0.0, f64::max))
            .collect();
        let mut rank_y = vec![0.0; nranks];
        let mut y = 0.0;
        for r in 0..nranks {
            rank_y[r] = y;
            y += rank_h[r] + gap_after[r];
        }
        let sep = |a: usize, b: usize| -> f64 {
            let gap = if is_dummy[a] || is_dummy[b] {
                DUMMY_SEP
            } else {
                NODE_SEP
            };
            node_w[a] / 2.0 + gap + node_w[b] / 2.0
        };
        let mut cx = vec![0.0f64; total];
        for layer in &layers {
            let mut x = 0.0;
            for (i, &v) in layer.iter().enumerate() {
                if i > 0 {
                    x += sep(layer[i - 1], v);
                } else {
                    x = node_w[v] / 2.0;
                }
                cx[v] = x;
            }
        }
        for iter in 0..12 {
            let down = iter % 2 == 0;
            let order: Vec<usize> = if down {
                (0..nranks).collect()
            } else {
                (0..nranks).rev().collect()
            };
            for r in order {
                let layer = &layers[r];
                if layer.is_empty() {
                    continue;
                }
                let desired: Vec<f64> = layer
                    .iter()
                    .map(|&v| {
                        let mut nb = if down {
                            preds(v, &segs)
                        } else {
                            succs(v, &segs)
                        };
                        if nb.is_empty() {
                            nb = if down {
                                succs(v, &segs)
                            } else {
                                preds(v, &segs)
                            };
                        }
                        if nb.is_empty() {
                            cx[v]
                        } else {
                            nb.iter().map(|&u| cx[u]).sum::<f64>() / nb.len() as f64
                        }
                    })
                    .collect();
                let m = layer.len();
                let mut left = desired.clone();
                for i in 1..m {
                    left[i] = left[i].max(left[i - 1] + sep(layer[i - 1], layer[i]));
                }
                let mut right = desired.clone();
                for i in (0..m.saturating_sub(1)).rev() {
                    right[i] = right[i].min(right[i + 1] - sep(layer[i], layer[i + 1]));
                }
                for i in 0..m {
                    cx[layer[i]] = (left[i] + right[i]) / 2.0;
                }
            }
        }
        let min_x = (0..total)
            .map(|v| cx[v] - node_w[v] / 2.0)
            .fold(f64::INFINITY, f64::min);
        let min_x = if min_x.is_finite() { min_x } else { 0.0 };
        for v in cx.iter_mut() {
            *v -= min_x;
        }
        let width = (0..total)
            .map(|v| cx[v] + node_w[v] / 2.0)
            .fold(0.0, f64::max);
        let height = (0..nranks)
            .map(|r| rank_y[r] + rank_h[r])
            .fold(0.0, f64::max);
        let cy = |v: usize| rank_y[node_rank[v]] + rank_h[node_rank[v]] / 2.0;

        // 6. Map back to the requested direction.
        let to_dir = |x: f64, y: f64| -> (f64, f64) {
            match dir {
                Direction::Down => (x, y),
                Direction::Up => (x, height - y),
                Direction::Right => (y, x),
                Direction::Left => (height - y, x),
            }
        };
        for (li, &c) in children.iter().enumerate() {
            let (x, y) = to_dir(cx[li], cy(li));
            let (cw, ch) = self.size[c];
            self.rel[c] = (x - cw / 2.0, y - ch / 2.0);
        }
        for (ei, dummies, rev) in chains {
            let mut pts: Vec<(f64, f64)> = dummies.iter().map(|&d| to_dir(cx[d], cy(d))).collect();
            if rev {
                pts.reverse();
            }
            self.bends[ei] = pts;
        }
        if horizontal {
            (height, width)
        } else {
            (width, height)
        }
    }
}

/// Count edge crossings between adjacent layers.
fn crossings(layers: &[Vec<usize>], segs: &[(usize, usize)], total: usize) -> usize {
    let mut pos = vec![0usize; total];
    let mut layer_of = vec![0usize; total];
    for (r, l) in layers.iter().enumerate() {
        for (i, &v) in l.iter().enumerate() {
            pos[v] = i;
            layer_of[v] = r;
        }
    }
    let mut count = 0;
    for (i, a) in segs.iter().enumerate() {
        for b in &segs[i + 1..] {
            if layer_of[a.0] != layer_of[b.0] {
                continue;
            }
            let (a0, a1, b0, b1) = (pos[a.0], pos[a.1], pos[b.0], pos[b.1]);
            if (a0 < b0 && a1 > b1) || (a0 > b0 && a1 < b1) {
                count += 1;
            }
        }
    }
    count
}

/// Point halfway along a polyline.
pub fn midpoint(pts: &[(f64, f64)]) -> (f64, f64) {
    let total: f64 = pts
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum();
    let mut left = total / 2.0;
    for w in pts.windows(2) {
        let len = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        if len >= left && len > 0.0 {
            let t = left / len;
            return (
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            );
        }
        left -= len;
    }
    pts.first().copied().unwrap_or_default()
}

/// Where the ray from the center of `r` towards `toward` leaves the shape.
pub fn clip(shape: &str, r: Rect, toward: (f64, f64)) -> (f64, f64) {
    let (cx, cy) = r.center();
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return (cx, cy);
    }
    let (hw, hh) = (r.w / 2.0, r.h / 2.0);
    let t = match shape {
        "circle" | "oval" => 1.0 / ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt(),
        "diamond" => 1.0 / (dx.abs() / hw + dy.abs() / hh),
        _ => {
            let tx = if dx.abs() > 1e-9 {
                hw / dx.abs()
            } else {
                f64::INFINITY
            };
            let ty = if dy.abs() > 1e-9 {
                hh / dy.abs()
            } else {
                f64::INFINITY
            };
            tx.min(ty)
        }
    };
    let t = t.min(1.0);
    (cx + dx * t, cy + dy * t)
}

#[cfg(test)]
mod tests {
    use super::super::graph::compile;
    use super::super::parser::parse;
    use super::*;

    fn lay(src: &str) -> (Graph, Layout) {
        let g = compile(&parse(src).unwrap()).unwrap();
        let l = layout(&g);
        (g, l)
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn chain_goes_down() {
        let (_, l) = lay("a -> b -> c\n");
        assert!(l.boxes[1].y < l.boxes[2].y && l.boxes[2].y < l.boxes[3].y);
    }

    #[test]
    fn direction_right() {
        let (_, l) = lay("direction: right\na -> b\n");
        assert!(l.boxes[1].x + l.boxes[1].w < l.boxes[2].x);
    }

    #[test]
    fn siblings_do_not_overlap_and_children_stay_inside() {
        let (g, l) = lay("a -> b\na -> c\na -> d\ngrp: {x -> y; z}\nb -> grp.x\nd -> a\n");
        for i in 1..g.objects.len() {
            for j in i + 1..g.objects.len() {
                let (oi, oj) = (&g.objects[i], &g.objects[j]);
                if oi.parent == oj.parent {
                    assert!(
                        !overlap(l.boxes[i], l.boxes[j]),
                        "{} overlaps {}",
                        oi.abs,
                        oj.abs
                    );
                }
            }
            let p = g.objects[i].parent;
            if p != 0 {
                let (c, pb) = (l.boxes[i], l.boxes[p]);
                assert!(c.x >= pb.x && c.y >= pb.y);
                assert!(c.x + c.w <= pb.x + pb.w + 0.01 && c.y + c.h <= pb.y + pb.h + 0.01);
            }
        }
        assert_eq!(l.routes.len(), g.edges.len());
    }

    #[test]
    fn near_goes_above_and_grid_is_regular() {
        let (g, l) = lay("t: Title {near: top-center}\na -> b\ng: {grid-columns: 2; p; q; r}\n");
        let idx = |k: &str| g.objects.iter().position(|o| o.abs == k).unwrap();
        let (t, a) = (l.boxes[idx("t")], l.boxes[idx("a")]);
        assert!(t.y + t.h < a.y);
        let (p, q, r) = (
            l.boxes[idx("g.p")],
            l.boxes[idx("g.q")],
            l.boxes[idx("g.r")],
        );
        assert_eq!(p.y, q.y);
        assert!(q.x > p.x);
        assert_eq!(r.x, p.x);
        assert!(r.y > p.y);
        assert_eq!(p.w, r.w);
    }

    #[test]
    fn clip_rect() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        assert_eq!(clip("rectangle", r, (50.0, 500.0)), (50.0, 50.0));
        assert_eq!(clip("circle", r, (500.0, 25.0)), (100.0, 25.0));
    }
}
