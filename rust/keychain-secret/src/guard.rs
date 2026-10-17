//! A guard for an application's own test suite: no test may read or write
//! the real OS keychain. [`violations`] walks a repository and reports every
//! forbidden call inside test code — a file under a `tests/` directory, or a
//! `#[cfg(test)] mod … { … }` block anywhere else — so one test in the
//! application fails when a test reaches the keychain instead of a
//! [`MemoryStore`](crate::MemoryStore) or an injected lookup.
//!
//! Comments and string contents are blanked before the search, so a doc
//! comment or a string fixture that merely mentions a forbidden name never
//! counts as a call. Raw strings (`r#"…"#`) are not special-cased: the worst
//! a mismatch does is widen or shrink a masked span. Extracted from cox's
//! `crates/cox/tests/no_real_keychain_in_tests.rs`.
//!
//! ```no_run
//! #[test]
//! fn no_test_reads_the_real_keychain() {
//!     let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
//!     let mut forbidden = keychain_secret::guard::KEYCHAIN_CALLS.to_vec();
//!     forbidden.push("platform_secret("); // the application's own wrapper
//!     let found = keychain_secret::guard::violations(&root, &forbidden);
//!     assert!(found.is_empty(), "a test touches the real keychain:\n{}", found.join("\n"));
//! }
//! ```

use std::fs;
use std::path::Path;

/// Calls that reach the real keychain through this crate or keyring itself.
/// Add the application's own wrappers to the list it passes to
/// [`violations`].
pub const KEYCHAIN_CALLS: &[&str] = &["keyring::Entry", "Keychain::new(", "os_store("];

/// Directories never walked: build output, VCS data and dependencies.
const SKIP: &[&str] = &["target", "node_modules"];

/// Every match of `forbidden` inside test code under `root`, as
/// `"<path>: line N: `pattern`"`. Hidden directories, `target` and
/// `node_modules` are skipped; unreadable files are skipped too, since a
/// guard that cannot read a file has nothing to report about it.
pub fn violations(root: &Path, forbidden: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, false, forbidden, &mut out);
    out
}

fn walk(dir: &Path, in_tests: bool, forbidden: &[&str], out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            if !name.starts_with('.') && !SKIP.contains(&name) {
                walk(&path, in_tests || name == "tests", forbidden, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(src) = fs::read_to_string(&path)
        {
            for hit in violations_in(&src, in_tests, forbidden) {
                out.push(format!("{}: {hit}", path.display()));
            }
        }
    }
}

/// Every forbidden match inside `source`'s test regions, as
/// `"line N: `pattern`"`.
fn violations_in(source: &str, whole_file: bool, forbidden: &[&str]) -> Vec<String> {
    let masked = mask(source);
    let mut hits = Vec::new();
    for (start, end) in test_regions(&masked, whole_file) {
        let region = &masked[start..end];
        for pattern in forbidden {
            let mut from = 0;
            while let Some(rel) = region[from..].find(pattern) {
                let pos = start + from + rel;
                let line = masked[..pos].matches('\n').count() + 1;
                hits.push(format!("line {line}: `{pattern}`"));
                from += rel + pattern.len();
            }
        }
    }
    hits
}

/// `src` with every `//` and `/* */` comment and the contents of `"…"`
/// literals replaced by spaces, keeping line breaks and byte offsets.
fn mask(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let blank = |out: &mut Vec<u8>, i: usize| {
        if let Some(b) = out.get_mut(i)
            && *b != b'\n'
        {
            *b = b' ';
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1)) {
            (b'/', Some(b'/')) => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    blank(&mut out, i);
                    i += 1;
                }
            }
            (b'/', Some(b'*')) => {
                let start = i;
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                (start..i).for_each(|j| blank(&mut out, j));
            }
            // `'"'` is a character, not the start of a string.
            (b'\'', Some(b'"')) if bytes.get(i + 2) == Some(&b'\'') => i += 3,
            (b'"', _) => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    let step = if bytes[i] == b'\\' { 2 } else { 1 };
                    (i..i + step).for_each(|j| blank(&mut out, j));
                    i += step;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    // Only whole multi-byte characters were blanked or kept, so this is lossless.
    String::from_utf8_lossy(&out).into_owned()
}

/// The byte index of the `}` matching the `{` at `masked[open]`.
fn matching_brace(masked: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in masked.bytes().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Test regions in masked source: all of it for a file under `tests/`,
/// otherwise every `#[cfg(test)] mod <name> { … }` block, skipping any further
/// attributes between the two. A `#[cfg(test)]` item that is not a `mod` is
/// left alone rather than guessed at.
fn test_regions(masked: &str, whole_file: bool) -> Vec<(usize, usize)> {
    if whole_file {
        return vec![(0, masked.len())];
    }
    let bytes = masked.as_bytes();
    let marker = "#[cfg(test)]";
    let mut regions = Vec::new();
    let mut from = 0;
    while let Some(rel) = masked[from..].find(marker) {
        let attr_end = from + rel + marker.len();
        let mut j = attr_end;
        loop {
            while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                j += 1;
            }
            match masked[j..].find(']') {
                Some(close) if bytes.get(j) == Some(&b'#') => j += close + 1,
                _ => break,
            }
        }
        if masked[j..].starts_with("mod ")
            && let Some(brace) = masked[j..].find('{')
            && let Some(end) = matching_brace(masked, j + brace)
        {
            regions.push((j + brace, end + 1));
            from = end + 1;
        } else {
            from = attr_end;
        }
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALLS: &[&str] = &["keyring::Entry", "Keychain::new(", "platform_secret("];

    #[test]
    fn a_planted_call_in_a_tests_module_is_reported_with_its_line() {
        let src = "fn real() { let _ = Keychain::new(\"app\"); }\n#[cfg(test)]\n#[allow(dead_code)]\nmod tests {\n    fn x() {\n        let _ = platform_secret(\"s\");\n    }\n}\n";
        assert_eq!(
            violations_in(src, false, CALLS),
            ["line 6: `platform_secret(`"]
        );
    }

    #[test]
    fn comments_strings_and_lookalikes_do_not_count() {
        let src = "#[cfg(test)]\nmod tests {\n    // keyring::Entry in prose\n    /* Keychain::new( */\n    fn x() {\n        let _ = \"keyring::Entry \\\" Keychain::new(\";\n        let _ = platform_secret_with(\"s\", |_| None);\n    }\n}\n";
        assert!(violations_in(src, false, CALLS).is_empty());
    }

    #[test]
    fn a_file_under_tests_is_test_code_without_an_attribute() {
        let src = "fn helper() {\n    let _ = keyring::Entry::new(\"app\", \"x\");\n}\n";
        assert!(
            violations_in(src, false, CALLS).is_empty(),
            "not in a tests module"
        );
        assert_eq!(violations_in(src, true, CALLS).len(), 1);
    }

    #[test]
    fn a_quote_character_literal_does_not_open_a_string() {
        let src =
            "#[cfg(test)]\nmod tests {\n    fn x() { let q = b'\"'; Keychain::new(\"a\"); }\n}\n";
        assert_eq!(
            violations_in(src, false, CALLS),
            ["line 3: `Keychain::new(`"]
        );
    }

    #[test]
    fn non_ascii_text_keeps_lines_and_offsets() {
        let src = "#[cfg(test)]\nmod tests {\n    // ключ ü\n    fn x() { let _ = \"пароль\"; Keychain::new(\"a\"); }\n}\n";
        assert_eq!(
            violations_in(src, false, CALLS),
            ["line 4: `Keychain::new(`"]
        );
    }

    #[test]
    fn this_crate_keeps_its_own_tests_off_the_keychain() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = violations(root, KEYCHAIN_CALLS);
        assert!(found.is_empty(), "{}", found.join("\n"));
    }
}
