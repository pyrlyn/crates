//! A [ratatui] widget for render status and diagnostics (feature `tui`).
//!
//! No terminal image preview: d2 output is SVG (or PNG via d2), and showing
//! it in a terminal needs a rasteriser plus a graphics protocol crate, which
//! would dwarf this crate. The widget shows the output path instead.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::diagnostic::Diagnostic;
use crate::{Error, RenderReport, Severity};

/// What the widget shows.
#[derive(Debug, Clone, Default)]
pub enum RenderStatus {
    /// Nothing rendered yet.
    #[default]
    Idle,
    /// A render is running for this input.
    Rendering(PathBuf),
    /// The last render succeeded.
    Done(Box<RenderReport>),
    /// The last render failed.
    Failed {
        /// The input that failed.
        input: Option<PathBuf>,
        /// One-line error message.
        message: String,
        /// Located diagnostics.
        diagnostics: Vec<Diagnostic>,
    },
}

impl RenderStatus {
    /// Status from a render result.
    pub fn from_result(input: Option<PathBuf>, result: &Result<RenderReport, Error>) -> Self {
        match result {
            Ok(r) => RenderStatus::Done(Box::new(r.clone())),
            Err(e) => RenderStatus::Failed {
                input,
                message: e.to_string(),
                diagnostics: e.diagnostics().to_vec(),
            },
        }
    }

    /// The lines the widget draws (useful for tests and other UIs).
    pub fn lines(&self) -> Vec<Line<'static>> {
        let label =
            |s: &'static str| Span::styled(s, Style::default().add_modifier(Modifier::BOLD));
        match self {
            RenderStatus::Idle => vec![Line::from("waiting for a render")],
            RenderStatus::Rendering(p) => vec![Line::from(vec![
                Span::styled("rendering ", Style::default().fg(Color::Yellow)),
                Span::raw(p.display().to_string()),
            ])],
            RenderStatus::Done(r) => {
                let mut v = vec![
                    Line::from(vec![
                        Span::styled("ok ", Style::default().fg(Color::Green)),
                        Span::raw(format!("{} in {:.1?}", r.format, r.duration)),
                    ]),
                    Line::from(vec![
                        label("input  "),
                        Span::raw(r.input.display().to_string()),
                    ]),
                    Line::from(vec![
                        label("output "),
                        Span::raw(r.output.display().to_string()),
                    ]),
                ];
                if let Some(svg) = &r.svg {
                    v.push(Line::from(vec![
                        label("svg    "),
                        Span::raw(svg.to_string()),
                    ]));
                }
                for w in &r.warnings {
                    v.push(Line::from(Span::styled(
                        format!("warning: {w}"),
                        Style::default().fg(Color::Yellow),
                    )));
                }
                v
            }
            RenderStatus::Failed {
                input,
                message,
                diagnostics,
            } => {
                let mut v = vec![Line::from(Span::styled(
                    "failed",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ))];
                if let Some(i) = input {
                    v.push(Line::from(vec![
                        label("input  "),
                        Span::raw(i.display().to_string()),
                    ]));
                }
                if diagnostics.is_empty() {
                    v.push(Line::from(message.clone()));
                }
                for d in diagnostics {
                    let color = match d.severity {
                        Severity::Error => Color::Red,
                        Severity::Warning => Color::Yellow,
                    };
                    let pos = match (d.line, d.column) {
                        (Some(l), Some(c)) => format!("{l}:{c} "),
                        _ => String::new(),
                    };
                    v.push(Line::from(vec![
                        Span::styled(pos, Style::default().fg(color)),
                        Span::raw(d.message.clone()),
                    ]));
                }
                v
            }
        }
    }
}

/// Bordered panel showing a [`RenderStatus`].
#[derive(Debug, Clone)]
pub struct RenderStatusWidget<'a> {
    status: &'a RenderStatus,
    title: &'a str,
}

impl<'a> RenderStatusWidget<'a> {
    /// A widget titled "d2".
    pub fn new(status: &'a RenderStatus) -> Self {
        Self {
            status,
            title: "d2",
        }
    }

    /// Change the title.
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = title;
        self
    }
}

impl Widget for RenderStatusWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border = match self.status {
            RenderStatus::Failed { .. } => Color::Red,
            RenderStatus::Done(_) => Color::Green,
            _ => Color::Gray,
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title(self.title.to_string());
        Paragraph::new(self.status.lines())
            .block(block)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }
}

impl Widget for &RenderStatus {
    fn render(self, area: Rect, buf: &mut Buffer) {
        RenderStatusWidget::new(self).render(area, buf);
    }
}
