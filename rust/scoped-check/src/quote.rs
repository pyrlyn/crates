//! Quotes substituted values for the shell that runs the gates: POSIX `sh` through `shlex`,
//! `cmd.exe` on Windows, whose quoting `shlex`'s single quotes would not survive.

/// One quoted word for the platform shell.
pub fn quote(word: &str) -> Result<String, String> {
    if cfg!(windows) {
        cmd_quote(word)
    } else {
        shlex::try_quote(word)
            .map(String::from)
            .map_err(|e| e.to_string())
    }
}

/// `words` quoted for the platform shell and joined by spaces.
pub fn join<'a>(words: impl IntoIterator<Item = &'a str>) -> Result<String, String> {
    let quoted: Result<Vec<String>, String> = words.into_iter().map(quote).collect();
    Ok(quoted?.join(" "))
}

/// A word for `cmd /S /C "<command>"` (see `shell` in `main.rs`) whose program parses its
/// arguments by the MSVC rules. Inside double quotes cmd.exe keeps `&|<>()^` and spaces
/// literal, and `!` stays literal because delayed expansion is off. Nothing escapes `%` or `"`
/// for cmd.exe, and a line break ends the command, so such words are refused.
fn cmd_quote(word: &str) -> Result<String, String> {
    if let Some(c) = word
        .chars()
        .find(|&c| c == '"' || c == '%' || c.is_control())
    {
        return Err(format!(
            "{word:?} contains {c:?}, which cmd.exe cannot quote"
        ));
    }
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./:\\@+".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        return Ok(word.to_owned());
    }
    // Backslashes before the closing quote would escape it for the MSVC parser; doubled,
    // they stay literal.
    let trailing = word.len() - word.trim_end_matches('\\').len();
    Ok(format!("\"{word}{}\"", "\\".repeat(trailing)))
}

#[cfg(test)]
mod tests {
    use super::cmd_quote;

    #[test]
    fn cmd_leaves_plain_words_bare() {
        for word in ["README.md", "docs/a-b_c.md", "-p", "C:\\src\\x.rs", "a@b+c"] {
            assert_eq!(cmd_quote(word).unwrap(), word);
        }
    }

    #[test]
    fn cmd_double_quotes_metacharacters_and_spaces() {
        assert_eq!(cmd_quote("a b.md").unwrap(), "\"a b.md\"");
        assert_eq!(
            cmd_quote("docs/x & calc & .md").unwrap(),
            "\"docs/x & calc & .md\""
        );
        assert_eq!(
            cmd_quote("package(=a) | package(=b)").unwrap(),
            "\"package(=a) | package(=b)\""
        );
        assert_eq!(cmd_quote("a^b<c>!d").unwrap(), "\"a^b<c>!d\"");
        assert_eq!(cmd_quote("").unwrap(), "\"\"");
    }

    #[test]
    fn cmd_doubles_backslashes_before_the_closing_quote() {
        assert_eq!(cmd_quote("a b\\").unwrap(), "\"a b\\\\\"");
        assert_eq!(cmd_quote("a\\b c\\\\").unwrap(), "\"a\\b c\\\\\\\\\"");
    }

    #[test]
    fn cmd_refuses_what_it_cannot_quote() {
        for word in ["50%.md", "%PATH%", "a\"b", "a\nb", "a\rb", "a\0b"] {
            let err = cmd_quote(word).unwrap_err();
            assert!(err.contains("cmd.exe cannot quote"), "{err}");
        }
    }
}
