use std::path::PathBuf;
use std::time::Duration;

use d2_render::tui::{RenderStatus, RenderStatusWidget};
use d2_render::{Diagnostic, Error, Format, RenderReport};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

fn draw(status: &RenderStatus) -> String {
    let area = Rect::new(0, 0, 60, 8);
    let mut buf = Buffer::empty(area);
    RenderStatusWidget::new(status)
        .title("diagram")
        .render(area, &mut buf);
    let mut s = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

#[test]
fn shows_success() {
    let report = RenderReport {
        input: PathBuf::from("in.d2"),
        output: PathBuf::from("out/in.svg"),
        format: Format::Svg,
        duration: Duration::from_millis(12),
        backend: "cli",
        warnings: vec!["minor".into()],
        diagnostics: vec![],
        svg: None,
        source_hash: String::new(),
        stderr: String::new(),
    };
    let text = draw(&RenderStatus::Done(Box::new(report)));
    assert!(text.contains("diagram"));
    assert!(text.contains("out/in.svg"));
    assert!(text.contains("warning: minor"));
}

#[test]
fn shows_diagnostics() {
    let err = Error::Syntax {
        diagnostics: vec![Diagnostic::error(5, 4, "maps must be terminated with }")],
        stderr: String::new(),
    };
    let status = RenderStatus::from_result(Some("bad.d2".into()), &Err(err));
    let text = draw(&status);
    assert!(text.contains("failed"));
    assert!(text.contains("5:4 maps must be terminated with }"));
}

#[test]
fn idle_and_rendering() {
    assert!(draw(&RenderStatus::Idle).contains("waiting"));
    assert!(draw(&RenderStatus::Rendering("x.d2".into())).contains("rendering x.d2"));
}
