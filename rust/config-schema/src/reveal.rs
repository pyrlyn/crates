//! Uncommenting a documented default before it is edited, for config files
//! that start life as a reference file with every default commented out
//! (`# port = 8790  # the listen port`).

/// A generated reference file comments its defaults out. Reveals each
/// documented assignment on the path of `dotted`, so [`assign`] edits that line
/// in place and keeps its padding and trailing comment instead of appending a
/// second key. An intermediate scalar is revealed too, so setting below one is
/// reported as "not a table" rather than inventing a table next to a comment.
/// A key that is already set is left alone: uncommenting would duplicate it.
pub(crate) fn reveal_commented_key(text: &str, dotted: &str) -> String {
    let parts: Vec<&str> = dotted.split('.').collect();
    let mut out = text.to_string();
    for (i, leaf) in parts.iter().enumerate() {
        let table = (i > 0).then(|| parts[..i].join("."));
        if !live_key_present(&out, table.as_deref(), leaf) {
            out = reveal_assignment(&out, table.as_deref(), leaf);
        }
    }
    out
}

/// Whether `line` sits in the table `want` (`None` is the root).
fn in_table(current: &Option<String>, want: Option<&str>) -> bool {
    match want {
        None => current.is_none(),
        Some(want) => current.as_deref() == Some(want),
    }
}

fn reveal_assignment(text: &str, table: Option<&str>, leaf: &str) -> String {
    let mut current: Option<String> = None;
    let mut revealed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let raw = line.trim_end_matches(['\r', '\n']);
        if let Some(header) = table_header(raw.trim()) {
            current = Some(header);
        } else if !revealed && in_table(&current, table) {
            if let Some(bare) = reveal_line(raw, leaf) {
                revealed = true;
                out.push_str(&bare);
                out.push_str(&line[raw.len()..]);
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

fn live_key_present(text: &str, table: Option<&str>, leaf: &str) -> bool {
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(header) = table_header(trimmed) {
            current = Some(header);
            continue;
        }
        if !in_table(&current, table) || trimmed.starts_with('#') {
            continue;
        }
        let code = trimmed.split('#').next().unwrap_or("").trim();
        if code
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == leaf)
        {
            return true;
        }
    }
    false
}

fn table_header(trimmed: &str) -> Option<String> {
    let code = trimmed.split('#').next()?.trim();
    let inner = code.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() || inner.contains('[') {
        return None;
    }
    Some(inner.to_string())
}

fn reveal_line(raw: &str, leaf: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    let indent = raw.len() - trimmed.len();
    let rest = trimmed.strip_prefix("# ")?;
    let code = rest.split('#').next()?.trim();
    let key = code.split_once('=')?.0.trim();
    if key != leaf {
        return None;
    }
    Some(format!("{}{rest}", &raw[..indent]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commented_key_in_another_table_is_not_revealed() {
        let text = "[other]\n# port = 1\n";
        let result = reveal_commented_key(text, "port");
        assert_eq!(result, text);
        assert_eq!(
            reveal_commented_key("# port = 1\r\n", "port"),
            "port = 1\r\n"
        );
    }
}
