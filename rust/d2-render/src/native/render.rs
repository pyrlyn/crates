// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// SVG output modelled on D2's SVG renderer (d2renderers/d2svg) and shape
// outlines (lib/shape), Copyright 2022 Terrastruct, Inc.

//! SVG writer for a laid-out graph.

use std::fmt::Write as _;

use super::graph::{ArrowShape, Arrowhead, Graph, Style};
use super::layout::{Layout, Rect, CONTAINER_ICON, ICON_SIZE};
use super::theme::{Theme, THEMES};
use crate::svg::{base64_encode, escape_xml};

/// Rendering knobs the native backend honours.
#[derive(Debug, Clone)]
pub struct SvgOptions {
    /// Padding around the diagram.
    pub pad: f64,
    /// Emit `<?xml ...?>`.
    pub xml_tag: bool,
    /// Emit the generator version attribute.
    pub version: bool,
    /// Suffix for ids.
    pub salt: String,
    /// Theme for default colors.
    pub theme: &'static Theme,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self {
            pad: 100.0,
            xml_tag: true,
            version: true,
            salt: String::new(),
            theme: &THEMES[0],
        }
    }
}

const FONT_FAMILY: &str = "'Source Sans Pro', 'Helvetica Neue', Arial, sans-serif";
const MONO_FAMILY: &str = "'Source Code Pro', Menlo, Consolas, monospace";

fn f(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// Text attributes for [`text_el`].
struct TextStyle<'a> {
    size: f64,
    color: &'a str,
    bold: bool,
    italic: bool,
    underline: bool,
    mono: bool,
}

impl TextStyle<'_> {
    fn of<'a>(st: &Style, size: f64, color: &'a str, bold: bool, italic: bool) -> TextStyle<'a> {
        TextStyle {
            size,
            color,
            bold: st.bold.unwrap_or(bold),
            italic: st.italic.unwrap_or(italic),
            underline: st.underline.unwrap_or(false),
            mono: st.font.as_deref() == Some("mono"),
        }
    }
}

/// Key of one arrowhead marker: shape, filled, stroke colour.
type MarkerKey = (ArrowShape, bool, String);

fn head_filled(h: &Arrowhead) -> bool {
    h.filled
        .unwrap_or(matches!(h.shape, ArrowShape::Triangle | ArrowShape::Arrow))
}

/// `<marker>` definition for an arrowhead.
fn marker_def(id: &str, key: &MarkerKey, bg: &str) -> String {
    let (shape, filled, color) = key;
    let c = escape_xml(color);
    let fill = if *filled { c.clone() } else { escape_xml(bg) };
    let (vw, vh, rx, w, h, body) = match shape {
        ArrowShape::Triangle => (
            10,
            10,
            9,
            10,
            10,
            format!("<path d=\"M0,0 L10,5 L0,10 z\" fill=\"{c}\"/>"),
        ),
        ArrowShape::Arrow => (
            10,
            10,
            9,
            11,
            11,
            format!("<path d=\"M0,0 L10,5 L0,10 L3,5 z\" fill=\"{c}\"/>"),
        ),
        ArrowShape::Diamond => (
            20,
            10,
            19,
            18,
            9,
            format!(
                "<path d=\"M1,5 L10,1 L19,5 L10,9 z\" fill=\"{fill}\" stroke=\"{c}\" \
                 stroke-width=\"1.5\"/>"
            ),
        ),
        ArrowShape::Circle => (
            10,
            10,
            9,
            10,
            10,
            format!(
                "<circle cx=\"5\" cy=\"5\" r=\"4\" fill=\"{fill}\" stroke=\"{c}\" stroke-width=\"1.5\"/>"
            ),
        ),
        ArrowShape::Box => (
            10,
            10,
            9,
            10,
            10,
            format!(
                "<rect x=\"1\" y=\"1\" width=\"8\" height=\"8\" fill=\"{fill}\" stroke=\"{c}\" \
                 stroke-width=\"1.5\"/>"
            ),
        ),
        ArrowShape::Cross => (
            10,
            10,
            5,
            10,
            10,
            format!(
                "<path d=\"M1,1 L9,9 M9,1 L1,9\" fill=\"none\" stroke=\"{c}\" stroke-width=\"1.5\"/>"
            ),
        ),
        ArrowShape::CfOne
        | ArrowShape::CfOneRequired
        | ArrowShape::CfMany
        | ArrowShape::CfManyRequired => {
            let mut d = String::new();
            match shape {
                ArrowShape::CfOne => d.push_str("M10,2 L10,14"),
                ArrowShape::CfOneRequired => d.push_str("M8,2 L8,14 M12,2 L12,14"),
                ArrowShape::CfMany => d.push_str("M6,8 L16,1 M6,8 L16,15 M6,8 L16,8"),
                _ => d.push_str("M6,8 L16,1 M6,8 L16,15 M6,8 L16,8 M3,2 L3,14"),
            }
            (
                16,
                16,
                16,
                16,
                16,
                format!("<path d=\"{d}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"1.5\"/>"),
            )
        }
    };
    format!(
        "<marker id=\"{id}\" viewBox=\"0 0 {vw} {vh}\" refX=\"{rx}\" refY=\"{ry}\" \
         markerWidth=\"{w}\" markerHeight=\"{h}\" markerUnits=\"userSpaceOnUse\" \
         orient=\"auto-start-reverse\">{body}</marker>",
        ry = f(vh as f64 / 2.0),
    )
}

/// Point `dist` along the route from its start (or end) plus a
/// perpendicular offset, for arrowhead labels.
fn along(pts: &[(f64, f64)], from_end: bool, dist: f64, side: f64) -> (f64, f64) {
    let (a, b) = if from_end {
        (pts[pts.len() - 1], pts[pts.len() - 2])
    } else {
        (pts[0], pts[1])
    };
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(1e-6);
    let (ux, uy) = (dx / len, dy / len);
    let d = dist.min(len / 2.0);
    (a.0 + ux * d - uy * side, a.1 + uy * d + ux * side)
}

/// Write the SVG document.
pub fn render(g: &Graph, l: &Layout, opts: &SvgOptions) -> String {
    let t = opts.theme;
    let salt = escape_xml(&opts.salt);
    let pad = opts.pad;
    let b = l.bounds;
    let (dx, dy) = (pad - b.x, pad - b.y);
    let width = b.w + 2.0 * pad;
    let height = b.h + 2.0 * pad;
    let mut s = String::new();
    if opts.xml_tag {
        s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>");
    }
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"{} \
         preserveAspectRatio=\"xMinYMin meet\" viewBox=\"0 0 {w} {h}\">",
        if opts.version {
            format!(" data-d2-render-native=\"{}\"", env!("CARGO_PKG_VERSION"))
        } else {
            String::new()
        },
        w = f(width),
        h = f(height),
    );
    let bg = g.objects[0]
        .style
        .fill
        .clone()
        .unwrap_or_else(|| t.n[6].to_string());
    let _ = write!(
        s,
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\" stroke-width=\"0\"/>",
        f(width),
        f(height),
        escape_xml(&bg)
    );

    // Arrowhead markers, one per (shape, filled, colour).
    let mut markers: Vec<MarkerKey> = Vec::new();
    let key_of = |h: &Arrowhead, stroke: &str| (h.shape, head_filled(h), stroke.to_string());
    for e in &g.edges {
        let stroke = e.style.stroke.clone().unwrap_or_else(|| t.b[0].to_string());
        for h in [&e.src_head, &e.dst_head]
            .into_iter()
            .flatten()
            .filter(|h| h.drawn)
        {
            let k = key_of(h, &stroke);
            if !markers.contains(&k) {
                markers.push(k);
            }
        }
    }
    if !markers.is_empty() {
        s.push_str("<defs>");
        for (i, k) in markers.iter().enumerate() {
            s.push_str(&marker_def(&format!("d2n-arrow-{i}{salt}"), k, &bg));
        }
        s.push_str("</defs>");
    }
    let _ = write!(s, "<g transform=\"translate({},{})\">", f(dx), f(dy));

    // Shapes: parents before children.
    let mut order = vec![0usize];
    let mut k = 0;
    while k < order.len() {
        let i = order[k];
        order.extend(g.objects[i].children.iter().copied());
        k += 1;
    }
    for &i in order.iter().skip(1) {
        shape(&mut s, g, l, t, i);
    }
    for (ei, e) in g.edges.iter().enumerate() {
        let r = &l.routes[ei];
        let stroke = e.style.stroke.clone().unwrap_or_else(|| t.b[0].to_string());
        let marker = |h: &Option<Arrowhead>, attr: &str| match h {
            Some(h) if h.drawn => {
                let k = key_of(h, &stroke);
                let mi = markers.iter().position(|m| *m == k).unwrap_or(0);
                format!(" {attr}=\"url(#d2n-arrow-{mi}{salt})\"")
            }
            _ => String::new(),
        };
        let sw = e.style.stroke_width.unwrap_or(2.0);
        if let Some(link) = &e.link {
            let _ = write!(s, "<a href=\"{}\" xlink:href=\"{0}\">", escape_xml(link));
        }
        let _ = write!(
            s,
            "<g class=\"{}\"{}>",
            class_attr(&e.key, &e.classes),
            opacity_attr(&e.style)
        );
        if let Some(tip) = &e.tooltip {
            let _ = write!(s, "<title>{}</title>", escape_xml(tip));
        }
        let _ = write!(
            s,
            "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"{}{}{}/>",
            path_d(&r.points),
            escape_xml(&stroke),
            f(sw),
            dash_attr(&e.style, sw),
            marker(&e.src_head, "marker-start"),
            marker(&e.dst_head, "marker-end"),
        );
        let label_color = e
            .style
            .font_color
            .clone()
            .unwrap_or_else(|| t.n[1].to_string());
        if let (Some(text), Some(lb)) = (&e.label, r.label) {
            let _ = write!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" rx=\"2\"/>",
                f(lb.x),
                f(lb.y),
                f(lb.w),
                f(lb.h),
                escape_xml(&bg)
            );
            let ts = TextStyle::of(
                &e.style,
                e.style.font_size.unwrap_or(16.0),
                &label_color,
                false,
                true,
            );
            text_el(&mut s, text, lb.x + lb.w / 2.0, lb.y, &ts);
        }
        if r.points.len() >= 2 {
            for (h, from_end) in [(&e.src_head, false), (&e.dst_head, true)] {
                if let Some(text) = h.as_ref().and_then(|h| h.label.as_deref()) {
                    let (x, y) = along(&r.points, from_end, 24.0, 14.0);
                    let ts = TextStyle::of(&Style::default(), 14.0, &label_color, false, false);
                    text_el(&mut s, text, x, y - 10.0, &ts);
                }
            }
        }
        s.push_str("</g>");
        if e.link.is_some() {
            s.push_str("</a>");
        }
    }
    s.push_str("</g></svg>");
    s
}

/// D2's group class: the base64 element key, then the D2 class names.
fn class_attr(key: &str, classes: &[String]) -> String {
    let mut out = base64_encode(escape_xml(key).as_bytes());
    for c in classes {
        out.push(' ');
        out.push_str(&escape_xml(c));
    }
    out
}

fn opacity_attr(st: &Style) -> String {
    st.opacity
        .map(|o| format!(" opacity=\"{}\"", f(o)))
        .unwrap_or_default()
}

fn dash_attr(st: &Style, sw: f64) -> String {
    match st.stroke_dash {
        Some(d) if d > 0.0 => {
            let len = d * sw;
            format!(" stroke-dasharray=\"{},{}\"", f(len), f(len))
        }
        _ => String::new(),
    }
}

/// Straight segments for two points, smooth quadratic joins through bends.
fn path_d(pts: &[(f64, f64)]) -> String {
    let mut d = String::new();
    let Some(&(x0, y0)) = pts.first() else {
        return d;
    };
    let _ = write!(d, "M {} {}", f(x0), f(y0));
    if pts.len() == 2 {
        let _ = write!(d, " L {} {}", f(pts[1].0), f(pts[1].1));
        return d;
    }
    for i in 1..pts.len() - 1 {
        let (cx, cy) = pts[i];
        let (nx, ny) = pts[i + 1];
        let (ex, ey) = if i + 1 == pts.len() - 1 {
            (nx, ny)
        } else {
            ((cx + nx) / 2.0, (cy + ny) / 2.0)
        };
        let _ = write!(d, " Q {} {} {} {}", f(cx), f(cy), f(ex), f(ey));
    }
    d
}

fn text_el(s: &mut String, text: &str, cx: f64, top: f64, ts: &TextStyle) {
    let line_h = ts.size * super::text::LINE_HEIGHT;
    let _ = write!(
        s,
        "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"{}\" font-size=\"{}\" \
         text-anchor=\"middle\"{}{}{}>",
        f(cx),
        f(top + line_h * 0.75),
        escape_xml(ts.color),
        if ts.mono { MONO_FAMILY } else { FONT_FAMILY },
        f(ts.size),
        if ts.bold { " font-weight=\"bold\"" } else { "" },
        if ts.italic {
            " font-style=\"italic\""
        } else {
            ""
        },
        if ts.underline {
            " text-decoration=\"underline\""
        } else {
            ""
        },
    );
    for (i, line) in text.split('\n').enumerate() {
        if i == 0 {
            let _ = write!(s, "<tspan x=\"{}\">{}</tspan>", f(cx), escape_xml(line));
        } else {
            let _ = write!(
                s,
                "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                f(cx),
                f(line_h),
                escape_xml(line)
            );
        }
    }
    s.push_str("</text>");
}

/// SVG element(s) for a shape outline.
fn outline(shape: &str, r: Rect, radius: f64, attrs: &str) -> String {
    let Rect { x, y, w, h } = r;
    let poly = |pts: &[(f64, f64)]| {
        let p: Vec<String> = pts
            .iter()
            .map(|(a, b)| format!("{},{}", f(*a), f(*b)))
            .collect();
        format!("<polygon points=\"{}\"{attrs}/>", p.join(" "))
    };
    match shape {
        "circle" | "oval" => format!(
            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{attrs}/>",
            f(x + w / 2.0),
            f(y + h / 2.0),
            f(w / 2.0),
            f(h / 2.0)
        ),
        "diamond" => poly(&[
            (x + w / 2.0, y),
            (x + w, y + h / 2.0),
            (x + w / 2.0, y + h),
            (x, y + h / 2.0),
        ]),
        "hexagon" => {
            let i = (w / 4.0).min(h / 2.0);
            poly(&[
                (x + i, y),
                (x + w - i, y),
                (x + w, y + h / 2.0),
                (x + w - i, y + h),
                (x + i, y + h),
                (x, y + h / 2.0),
            ])
        }
        "parallelogram" => {
            let i = (w * 0.15).min(26.0);
            poly(&[(x + i, y), (x + w, y), (x + w - i, y + h), (x, y + h)])
        }
        "step" => {
            let i = (w * 0.15).min(30.0);
            poly(&[
                (x, y),
                (x + w - i, y),
                (x + w, y + h / 2.0),
                (x + w - i, y + h),
                (x, y + h),
                (x + i, y + h / 2.0),
            ])
        }
        "callout" => {
            let body = h - 20.0;
            poly(&[
                (x, y),
                (x + w, y),
                (x + w, y + body),
                (x + w * 0.45, y + body),
                (x + w * 0.3, y + h),
                (x + w * 0.3, y + body),
                (x, y + body),
            ])
        }
        "page" => {
            let c = (w.min(h) * 0.25).min(20.0);
            format!(
                "<path d=\"M {x0} {y0} L {x1} {y0} L {x2} {y1} L {x2} {y2} L {x0} {y2} Z \
                 M {x1} {y0} L {x1} {y1} L {x2} {y1}\"{attrs}/>",
                x0 = f(x),
                y0 = f(y),
                x1 = f(x + w - c),
                x2 = f(x + w),
                y1 = f(y + c),
                y2 = f(y + h),
            )
        }
        "document" => {
            let wave = 10.0;
            format!(
                "<path d=\"M {} {} L {} {} L {} {} C {} {} {} {} {} {} C {} {} {} {} {} {} Z\"{attrs}/>",
                f(x),
                f(y),
                f(x + w),
                f(y),
                f(x + w),
                f(y + h - wave),
                f(x + w * 0.75),
                f(y + h - 2.0 * wave),
                f(x + w * 0.6),
                f(y + h),
                f(x + w * 0.5),
                f(y + h - wave),
                f(x + w * 0.35),
                f(y + h - 2.0 * wave),
                f(x + w * 0.2),
                f(y + h + wave * 0.5),
                f(x),
                f(y + h - wave),
            )
        }
        "cylinder" => {
            let ry = (h * 0.1).min(12.0);
            format!(
                "<path d=\"M {x0} {ya} A {rx} {ry} 0 0 1 {x1} {ya} L {x1} {yb} A {rx} {ry} 0 0 1 \
                 {x0} {yb} Z M {x0} {ya} A {rx} {ry} 0 0 0 {x1} {ya}\"{attrs}/>",
                x0 = f(x),
                x1 = f(x + w),
                ya = f(y + ry),
                yb = f(y + h - ry),
                rx = f(w / 2.0),
                ry = f(ry),
            )
        }
        "queue" => {
            let rx = (w * 0.1).min(12.0);
            format!(
                "<path d=\"M {xa} {y0} L {xb} {y0} A {rx} {ry} 0 0 1 {xb} {y1} L {xa} {y1} \
                 A {rx} {ry} 0 0 1 {xa} {y0} Z M {xb} {y0} A {rx} {ry} 0 0 0 {xb} {y1}\"{attrs}/>",
                xa = f(x + rx),
                xb = f(x + w - rx),
                y0 = f(y),
                y1 = f(y + h),
                rx = f(rx),
                ry = f(h / 2.0),
            )
        }
        "stored_data" => {
            let c = (w * 0.1).min(15.0);
            format!(
                "<path d=\"M {xa} {y0} L {x1} {y0} Q {xb} {ym} {x1} {y1} L {xa} {y1} Q {x0} {ym} \
                 {xa} {y0} Z\"{attrs}/>",
                xa = f(x + c),
                x0 = f(x - c),
                x1 = f(x + w),
                xb = f(x + w - 2.0 * c),
                y0 = f(y),
                y1 = f(y + h),
                ym = f(y + h / 2.0),
            )
        }
        "package" => {
            let tab_w = (w * 0.4).min(60.0);
            let tab_h = 10.0;
            format!(
                "<path d=\"M {x0} {y0} L {xt} {y0} L {xt} {yt} L {x1} {yt} L {x1} {y1} L {x0} {y1} Z\"{attrs}/>",
                x0 = f(x),
                xt = f(x + tab_w),
                x1 = f(x + w),
                y0 = f(y),
                yt = f(y + tab_h),
                y1 = f(y + h),
            )
        }
        "cloud" => format!(
            "<path d=\"M {a} {b} C {c} {d} {e} {g} {h_} {i} C {j} {k} {l} {m} {n} {o} \
             C {p} {q} {r_} {s_} {t} {u} C {v} {w_} {x_} {y_} {a} {b} Z\"{attrs}/>",
            a = f(x + w * 0.2),
            b = f(y + h * 0.8),
            c = f(x - w * 0.05),
            d = f(y + h * 0.8),
            e = f(x),
            g = f(y + h * 0.35),
            h_ = f(x + w * 0.25),
            i = f(y + h * 0.3),
            j = f(x + w * 0.3),
            k = f(y - h * 0.05),
            l = f(x + w * 0.7),
            m = f(y - h * 0.05),
            n = f(x + w * 0.75),
            o = f(y + h * 0.3),
            p = f(x + w * 1.02),
            q = f(y + h * 0.3),
            r_ = f(x + w * 1.05),
            s_ = f(y + h * 0.8),
            t = f(x + w * 0.8),
            u = f(y + h * 0.8),
            v = f(x + w * 0.7),
            w_ = f(y + h * 1.05),
            x_ = f(x + w * 0.3),
            y_ = f(y + h * 1.05),
        ),
        "person" | "c4-person" => {
            let head = (w.min(h - 30.0) * 0.22).max(8.0);
            let cx = x + w / 2.0;
            let body_top = y + 2.0 * head + 6.0;
            let body_bottom = y + h - 30.0;
            format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\"{attrs}/><path d=\"M {} {} Q {} {} {} {} \
                 L {} {} Q {} {} {} {} Z\"{attrs}/>",
                f(cx),
                f(y + head),
                f(head),
                f(x + w * 0.1),
                f(body_bottom),
                f(x + w * 0.1),
                f(body_top),
                f(cx),
                f(body_top),
                f(cx),
                f(body_top),
                f(x + w * 0.9),
                f(body_top),
                f(x + w * 0.9),
                f(body_bottom),
            )
        }
        _ => format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{}{attrs}/>",
            f(x),
            f(y),
            f(w),
            f(h),
            if radius > 0.0 {
                format!(" rx=\"{}\" ry=\"{}\"", f(radius), f(radius))
            } else {
                String::new()
            }
        ),
    }
}

fn image_el(s: &mut String, href: &str, x: f64, y: f64, size: f64) {
    let _ = write!(
        s,
        "<image href=\"{h}\" xlink:href=\"{h}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
        f(x),
        f(y),
        f(size),
        f(size),
        h = escape_xml(href),
    );
}

/// Top and side faces of a `3d` rectangle.
fn three_d(r: Rect, attrs: &str) -> String {
    let d = 15.0;
    let Rect { x, y, w, h } = r;
    let pts = |p: &[(f64, f64)]| {
        p.iter()
            .map(|(a, b)| format!("{},{}", f(*a), f(*b)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "<polygon points=\"{}\"{attrs}/><polygon points=\"{}\"{attrs}/>",
        pts(&[(x, y), (x + d, y - d), (x + w + d, y - d), (x + w, y)]),
        pts(&[
            (x + w, y),
            (x + w + d, y - d),
            (x + w + d, y + h - d),
            (x + w, y + h)
        ]),
    )
}

fn shape(s: &mut String, g: &Graph, l: &Layout, t: &Theme, i: usize) {
    let o = &g.objects[i];
    let r = l.boxes[i];
    let st = &o.style;
    let fs = l.font_sizes[i];
    if let Some(link) = &o.link {
        let _ = write!(s, "<a href=\"{}\" xlink:href=\"{0}\">", escape_xml(link));
    }
    let _ = write!(
        s,
        "<g class=\"{}\"{}>",
        class_attr(&o.abs, &o.classes),
        opacity_attr(st)
    );
    if let Some(tip) = &o.tooltip {
        let _ = write!(s, "<title>{}</title>", escape_xml(tip));
    }
    let is_image = o.shape == "image";
    if o.shape != "text" && !is_image {
        let dashed = st.stroke_dash.is_some_and(|d| d > 0.0);
        let fill = st.fill.clone().unwrap_or_else(|| t.fill(g, i).to_string());
        let stroke = st
            .stroke
            .clone()
            .unwrap_or_else(|| t.stroke(g, i, dashed).to_string());
        let sw = st.stroke_width.unwrap_or(2.0);
        let attrs = format!(
            " fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\"{}",
            escape_xml(&fill),
            escape_xml(&stroke),
            f(sw),
            dash_attr(st, sw)
        );
        let radius = st.border_radius.unwrap_or(0.0);
        s.push_str("<g class=\"shape\">");
        if st.shadow == Some(true) {
            let sh = Rect {
                x: r.x + 4.0,
                y: r.y + 4.0,
                ..r
            };
            s.push_str(&outline(
                &o.shape,
                sh,
                radius,
                " fill=\"#000000\" fill-opacity=\"0.12\" stroke=\"none\"",
            ));
        }
        if st.multiple == Some(true) {
            let back = Rect {
                x: r.x + 10.0,
                y: r.y - 10.0,
                ..r
            };
            s.push_str(&outline(&o.shape, back, radius, &attrs));
        }
        if st.three_d == Some(true) && matches!(o.shape.as_str(), "rectangle" | "square") {
            s.push_str(&three_d(r, &attrs));
        }
        s.push_str(&outline(&o.shape, r, radius, &attrs));
        if st.double_border == Some(true) && r.w > 20.0 && r.h > 20.0 {
            let inner = Rect {
                x: r.x + 5.0,
                y: r.y + 5.0,
                w: r.w - 10.0,
                h: r.h - 10.0,
            };
            s.push_str(&outline(&o.shape, inner, radius, &attrs));
        }
        s.push_str("</g>");
    }
    let label = o.label_text();
    let mono = o.shape == "code" || st.font.as_deref() == Some("mono");
    let (_, th) = if mono {
        super::text::measure_mono(label, fs)
    } else {
        super::text::measure(label, fs, true)
    };
    // Icon / image placement; the label moves below a leaf's icon.
    let mut label_top = None;
    let mut label_cx = r.x + r.w / 2.0;
    if let Some(icon) = &o.icon {
        if is_image {
            let size = r.w.min(r.h - th - 8.0).max(8.0);
            image_el(s, icon, r.x + (r.w - size) / 2.0, r.y, size);
            label_top = Some(r.y + size + 8.0);
        } else if o.is_container() {
            // Icon left of the centred label.
            let (lw, _) = super::text::measure(label, fs, true);
            let block = CONTAINER_ICON + 12.0 + lw;
            let x0 = r.x + (r.w - block) / 2.0;
            image_el(
                s,
                icon,
                x0,
                r.y + 15.0 + (th - CONTAINER_ICON) / 2.0,
                CONTAINER_ICON,
            );
            label_cx = x0 + CONTAINER_ICON + 12.0 + lw / 2.0;
        } else if o.shape != "text" {
            let block = ICON_SIZE + 8.0 + if label.is_empty() { 0.0 } else { th };
            let top = r.y + (r.h - block) / 2.0;
            image_el(s, icon, r.x + (r.w - ICON_SIZE) / 2.0, top, ICON_SIZE);
            label_top = Some(top + ICON_SIZE + 8.0);
        }
    }
    if !label.is_empty() {
        let color = st.font_color.clone().unwrap_or_else(|| t.n[0].to_string());
        let mut ts = TextStyle::of(st, fs, &color, o.shape != "text", false);
        ts.mono = mono;
        let top = if let Some(top) = label_top {
            top
        } else if o.is_container() {
            r.y + 15.0
        } else if o.shape == "person" || o.shape == "c4-person" {
            r.y + r.h - th - 2.0
        } else if o.shape == "cylinder" {
            r.y + (r.h - th) / 2.0 + 6.0
        } else {
            r.y + (r.h - th) / 2.0
        };
        text_el(s, label, label_cx, top, &ts);
    }
    s.push_str("</g>");
    if o.link.is_some() {
        s.push_str("</a>");
    }
}
