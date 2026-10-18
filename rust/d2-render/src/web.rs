//! HTML embedding helpers (feature `web`).

use std::fs;
use std::path::Path;

use crate::svg::{base64_encode, escape_xml};
use crate::{Format, Result};

/// Strip the XML declaration and any doctype so the SVG can sit inside HTML.
pub fn strip_xml_prolog(svg: &str) -> &str {
    let mut s = svg.trim_start();
    loop {
        if s.starts_with("<?") {
            match s.find("?>") {
                Some(i) => s = s[i + 2..].trim_start(),
                None => return s,
            }
        } else if s.starts_with("<!DOCTYPE") || s.starts_with("<!doctype") {
            match s.find('>') {
                Some(i) => s = s[i + 1..].trim_start(),
                None => return s,
            }
        } else {
            return s;
        }
    }
}

/// `<div class="d2-diagram">…inline svg…</div>`; `class` defaults to
/// `d2-diagram`. Several d2 SVGs on one page need distinct `--salt`s
/// ([`crate::RenderOptions::salt`]) so their ids do not clash.
pub fn inline_svg(svg: &str, class: Option<&str>) -> String {
    format!(
        "<div class=\"{}\">{}</div>",
        escape_xml(class.unwrap_or("d2-diagram")),
        strip_xml_prolog(svg).trim_end()
    )
}

/// [`inline_svg`] from a file.
pub fn inline_svg_file(path: &Path, class: Option<&str>) -> Result<String> {
    Ok(inline_svg(&fs::read_to_string(path)?, class))
}

/// `data:<mime>;base64,...` for any rendered bytes.
pub fn data_uri(bytes: &[u8], format: Format) -> String {
    format!("data:{};base64,{}", format.mime(), base64_encode(bytes))
}

/// [`data_uri`] from a file, with the format taken from its extension.
pub fn data_uri_file(path: &Path) -> Result<String> {
    let format = Format::from_path(path)?;
    Ok(data_uri(&fs::read(path)?, format))
}

/// `<img src="data:..." alt="...">`.
pub fn img_tag(bytes: &[u8], format: Format, alt: &str) -> String {
    format!(
        "<img src=\"{}\" alt=\"{}\">",
        data_uri(bytes, format),
        escape_xml(alt)
    )
}

/// A minimal standalone HTML page around an SVG.
pub fn html_page(title: &str, svg: &str) -> String {
    format!(
        concat!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
            "<title>{title}</title>\n<style>\n",
            "body{{margin:0;padding:24px;font-family:system-ui,sans-serif;background:#fff}}\n",
            ".d2-diagram svg{{max-width:100%;height:auto}}\n",
            "</style>\n</head>\n<body>\n<h1>{title}</h1>\n{body}\n</body>\n</html>\n"
        ),
        title = escape_xml(title),
        body = inline_svg(svg, None)
    )
}

/// [`html_page`] from an SVG file, written to `output`.
pub fn write_html_page(title: &str, svg_path: &Path, output: &Path) -> Result<()> {
    let page = html_page(title, &fs::read_to_string(svg_path)?);
    fs::write(output, page)?;
    Ok(())
}
