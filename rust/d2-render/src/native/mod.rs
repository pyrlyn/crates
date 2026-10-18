// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// The native backend is a Rust rewrite of a subset of D2
// (parser, compiler, dagre-style layout, SVG renderer),
// Copyright 2022 Terrastruct, Inc. See the README for what is covered.

//! Pure-Rust backend for a subset of D2 (feature `native`, on by default).
//!
//! Supported: shapes (including `image` and icons), labels (block strings
//! shown as plain text), nested containers, `->` `<-` `<->` `--`
//! connections and chains, labels, arrowhead shapes and labels,
//! `direction`, `width`/`height`, `_` references, `vars` with `${...}`
//! substitution and `d2-config`, `classes`, `near` constants, grids,
//! tooltips, links, all 20 themes, and the common `style` keywords.
//! Everything else D2 accepts is reported as a warning and ignored, or
//! rejected with a diagnostic (imports, globs, `(a -> b)[0]` references,
//! variable spreads). Output is SVG (PNG with feature `png`); layout is a
//! dagre-like layered layout, so positions differ from the CLI's. The
//! README lists exactly what is and is not supported.

pub mod graph;
pub mod layout;
pub mod parser;
#[cfg(feature = "png")]
pub mod png;
pub mod render;
mod text;
pub mod theme;

use std::fs;
use std::path::Path;

use crate::backend::{Backend, BackendOutput};
use crate::diagnostic::Diagnostic;
use crate::{Error, Format, RenderOptions, Result};

/// SVG and warnings from an in-memory render.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeOutput {
    /// The SVG document.
    pub svg: String,
    /// Unsupported features that were ignored.
    pub warnings: Vec<String>,
}

/// Parse, lay out and draw `source`. Errors carry D2-style diagnostics.
pub fn render_svg(source: &str, options: &RenderOptions) -> Result<NativeOutput> {
    let ast = parser::parse(source).map_err(syntax)?;
    let mut g = graph::compile(&ast).map_err(syntax)?;
    let mut warnings = std::mem::take(&mut g.warnings);
    warnings.extend(option_warnings(options, &g.config));
    let theme_id = options.theme.or(g.config.theme_id).unwrap_or(0);
    let t = theme::find(theme_id).unwrap_or_else(|| {
        warnings.push(format!("theme {theme_id} does not exist; using theme 0"));
        &theme::THEMES[0]
    });
    warnings.extend(apply_theme(&mut g, t));
    apply_text_transform(&mut g);
    let l = layout::layout(&g);
    let pad = options.pad.map(f64::from).or(g.config.pad).unwrap_or(100.0);
    let svg = render::render(
        &g,
        &l,
        &render::SvgOptions {
            pad,
            xml_tag: !options.no_xml_tag,
            version: !options.omit_version,
            salt: options.salt.clone().unwrap_or_default(),
            theme: t,
        },
    );
    Ok(NativeOutput { svg, warnings })
}

/// Apply the theme's special rules (`Mono`, `CapsLock`,
/// `OuterContainerDoubleBorder`); returns warnings for the others.
fn apply_theme(g: &mut graph::Graph, t: &theme::Theme) -> Vec<String> {
    if t.has("Mono") {
        let styles = g.objects.iter_mut().map(|o| &mut o.style);
        for st in styles.chain(g.edges.iter_mut().map(|e| &mut e.style)) {
            st.font.get_or_insert_with(|| "mono".into());
        }
    }
    if t.has("CapsLock") {
        for o in g.objects.iter_mut().skip(1) {
            if o.shape != "code" && o.style.text_transform.is_none() {
                o.style.text_transform = Some("uppercase".into());
            }
        }
    }
    if t.has("OuterContainerDoubleBorder") {
        for i in 1..g.objects.len() {
            if g.objects[i].level == 1 && g.objects[i].is_container() {
                g.objects[i].style.double_border.get_or_insert(true);
            }
        }
    }
    t.special
        .iter()
        .filter(|r| {
            !matches!(
                **r,
                "Mono" | "CapsLock" | "OuterContainerDoubleBorder" | "NoCornerRadius"
            )
        })
        .map(|r| {
            format!(
                "theme {} rule {r} is not drawn by the native backend",
                t.name
            )
        })
        .collect()
}

/// Bake `style.text-transform` into labels so layout measures them.
fn apply_text_transform(g: &mut graph::Graph) {
    let transform = |text: &str, how: &str| match how {
        "uppercase" => text.to_uppercase(),
        "lowercase" => text.to_lowercase(),
        "title" => text
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect::<String>())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => text.to_string(),
    };
    for o in g.objects.iter_mut().skip(1) {
        if let Some(how) = o.style.text_transform.clone() {
            let text = transform(o.label_text(), &how);
            o.label = Some(text);
        }
    }
    for e in &mut g.edges {
        if let (Some(how), Some(l)) = (e.style.text_transform.clone(), e.label.as_mut()) {
            *l = transform(l, &how);
        }
    }
}

/// Parse and compile without drawing.
pub fn check(source: &str) -> Result<()> {
    let ast = parser::parse(source).map_err(syntax)?;
    graph::compile(&ast).map_err(syntax)?;
    Ok(())
}

fn syntax(diagnostics: Vec<Diagnostic>) -> Error {
    let stderr = diagnostics.iter().map(|d| format!("err: {d}\n")).collect();
    Error::Syntax {
        diagnostics,
        stderr,
    }
}

fn with_path(e: Error, path: &Path) -> Error {
    match e {
        Error::Syntax {
            mut diagnostics, ..
        } => {
            for d in &mut diagnostics {
                d.path = Some(path.to_path_buf());
            }
            let stderr = diagnostics.iter().map(|d| format!("err: {d}\n")).collect();
            Error::Syntax {
                diagnostics,
                stderr,
            }
        }
        other => other,
    }
}

fn option_warnings(o: &RenderOptions, cfg: &graph::Config) -> Vec<String> {
    let mut w = Vec::new();
    let mut ignored = |name: &str| {
        w.push(format!("option {name} is ignored by the native backend"));
    };
    if o.dark_theme.or(cfg.dark_theme_id).is_some() {
        ignored("--dark-theme");
    }
    let layout_engine = o
        .layout
        .as_ref()
        .map(|l| *l != crate::Layout::Dagre)
        .or(cfg.layout_engine.as_ref().map(|l| l != "dagre"));
    if layout_engine == Some(true) {
        ignored("--layout (a dagre-like layout is always used)");
    }
    if o.sketch || cfg.sketch == Some(true) {
        ignored("--sketch");
    }
    if o.center || cfg.center == Some(true) {
        ignored("--center");
    }
    if o.target.is_some() {
        ignored("--target");
    }
    if o.animate_interval_ms.is_some() {
        ignored("--animate-interval");
    }
    w
}

/// In-process renderer for a subset of D2: SVG, plus PNG with feature
/// `png`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeBackend;

impl NativeBackend {
    /// A new backend (stateless).
    pub fn new() -> Self {
        NativeBackend
    }
}

impl Backend for NativeBackend {
    fn name(&self) -> &'static str {
        "native"
    }

    fn supports(&self, format: Format) -> bool {
        format == Format::Svg || (cfg!(feature = "png") && format == Format::Png)
    }

    fn render(
        &self,
        input: &Path,
        output: &Path,
        format: Format,
        options: &RenderOptions,
    ) -> Result<BackendOutput> {
        if !self.supports(format) {
            return Err(Error::UnsupportedFormat {
                format,
                backend: self.name(),
            });
        }
        let source = fs::read_to_string(input)?;
        let mut out = render_svg(&source, options).map_err(|e| with_path(e, input))?;
        match format {
            #[cfg(feature = "png")]
            Format::Png => {
                let dir = input.parent().filter(|p| !p.as_os_str().is_empty());
                let png = png::svg_to_png(&out.svg, options.scale.unwrap_or(2.0), dir)?;
                fs::write(output, png)?;
            }
            _ => {
                if options.scale.is_some() {
                    out.warnings
                        .push("option --scale is ignored by the native backend for SVG".into());
                }
                fs::write(output, out.svg)?;
            }
        }
        Ok(BackendOutput {
            stderr: out
                .warnings
                .iter()
                .map(|w| format!("warn: {w}\n"))
                .collect(),
            warnings: out.warnings,
            diagnostics: Vec::new(),
            version: Some(self.version()?),
        })
    }

    fn validate(&self, input: &Path) -> Result<()> {
        let source = fs::read_to_string(input)?;
        check(&source).map_err(|e| with_path(e, input))
    }

    fn version(&self) -> Result<String> {
        Ok(format!("d2-render-native v{}", env!("CARGO_PKG_VERSION")))
    }
}
