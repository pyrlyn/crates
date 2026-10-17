//! Draw the status widget into an off-screen buffer and print it.
//!
//! cargo run -p d2-render --features tui --example tui_report

use std::path::PathBuf;

use d2_render::tui::{RenderStatus, RenderStatusWidget};
use d2_render::{Format, Renderer};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

fn main() {
    let input = PathBuf::from("inline.d2");
    let renderer = Renderer::new();
    let result = renderer.render_str_report(
        "a -> b: ok\nb -> c: {",
        &PathBuf::from("target/tui.svg"),
        Format::Svg,
    );
    let status = RenderStatus::from_result(Some(input), &result);

    let area = Rect::new(0, 0, 72, 8);
    let mut buf = Buffer::empty(area);
    RenderStatusWidget::new(&status)
        .title("d2-render")
        .render(area, &mut buf);
    for y in 0..area.height {
        let line: String = (0..area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        println!("{line}");
    }
}
