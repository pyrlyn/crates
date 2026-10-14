// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! What a command would change, shown before it changes anything.
//!
//! File edits read as the unified diff `git diff` prints, or as `git diff --stat` lines; a
//! removal reads as one `- path  size  N files` line, because listing every file of a build
//! cache would be millions of lines. Either way the preview ends with a totals line. Shared so
//! rtok and ketch do not each grow their own dry-run format.

use std::path::{Path, PathBuf};

use owo_colors::{OwoColorize, Stream};
use serde::Serialize;
use similar::{ChangeTag, TextDiff};

/// How much of the preview to print.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Every edit as a unified diff.
    #[default]
    Diff,
    /// One `path | 7 +++--` line per edit instead of its diff.
    Stat,
}

/// One file the command would rewrite.
#[derive(Clone, Debug, Serialize)]
pub struct Edit {
    pub path: PathBuf,
    pub insertions: usize,
    pub deletions: usize,
    #[serde(skip)]
    diff: String,
}

/// One path the command would delete, with everything under it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Removal {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
}

/// The sums the closing lines print.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Totals {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths_removed: usize,
    pub files_removed: u64,
    pub bytes_removed: u64,
}

/// Everything one run would change.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Preview {
    pub edits: Vec<Edit>,
    pub removals: Vec<Removal>,
}

impl Preview {
    /// Record a rewrite of `path`; an edit that changes nothing is dropped.
    pub fn edit(&mut self, path: impl Into<PathBuf>, before: &str, after: &str) -> &mut Self {
        let path = path.into();
        if before == after {
            return self;
        }
        let (mut insertions, mut deletions) = (0, 0);
        for change in TextDiff::from_lines(before, after).iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => insertions += 1,
                ChangeTag::Delete => deletions += 1,
                ChangeTag::Equal => {}
            }
        }
        let diff = unified_diff(&path, before, after);
        self.edits.push(Edit {
            path,
            insertions,
            deletions,
            diff,
        });
        self
    }

    /// Record a deletion of `path`, which holds `files` files taking `bytes` bytes.
    pub fn remove(&mut self, path: impl Into<PathBuf>, bytes: u64, files: u64) -> &mut Self {
        self.removals.push(Removal {
            path: path.into(),
            bytes,
            files,
        });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty() && self.removals.is_empty()
    }

    pub fn totals(&self) -> Totals {
        Totals {
            files_changed: self.edits.len(),
            insertions: self.edits.iter().map(|e| e.insertions).sum(),
            deletions: self.edits.iter().map(|e| e.deletions).sum(),
            paths_removed: self.removals.len(),
            files_removed: self.removals.iter().map(|r| r.files).sum(),
            bytes_removed: self.removals.iter().map(|r| r.bytes).sum(),
        }
    }

    /// The preview as plain text; [`paint`] colours it for a terminal.
    pub fn render(&self, mode: Mode) -> String {
        let mut out = String::new();
        match mode {
            Mode::Diff => self.edits.iter().for_each(|e| out.push_str(&e.diff)),
            Mode::Stat => out.push_str(&self.stat_lines()),
        }
        out.push_str(&self.removal_lines());
        let t = self.totals();
        if t.files_changed > 0 {
            out.push_str(&edit_totals(&t));
            out.push('\n');
        }
        if t.paths_removed > 0 {
            out.push_str(&removal_totals(&t));
            out.push('\n');
        }
        out
    }

    fn stat_lines(&self) -> String {
        const BAR: usize = 40;
        let width = column(
            self.edits
                .iter()
                .map(|e| e.path.display().to_string().len()),
        );
        let count = column(
            self.edits
                .iter()
                .map(|e| (e.insertions + e.deletions).to_string().len()),
        );
        let most = self
            .edits
            .iter()
            .map(|e| e.insertions + e.deletions)
            .max()
            .unwrap_or(0);
        // A 2000-line rewrite must not print a 2000-character bar; past the cap every bar
        // shrinks by the same ratio, rounded up so a one-line change still shows a mark.
        let scale = |n: usize| {
            if most <= BAR {
                n
            } else {
                (n * BAR).div_ceil(most)
            }
        };
        self.edits
            .iter()
            .map(|e| {
                format!(
                    " {:<width$} | {:>count$} {}{}\n",
                    e.path.display(),
                    e.insertions + e.deletions,
                    "+".repeat(scale(e.insertions)),
                    "-".repeat(scale(e.deletions)),
                )
            })
            .collect()
    }

    fn removal_lines(&self) -> String {
        let width = column(
            self.removals
                .iter()
                .map(|r| r.path.display().to_string().len()),
        );
        let size = column(self.removals.iter().map(|r| human_bytes(r.bytes).len()));
        self.removals
            .iter()
            .map(|r| {
                format!(
                    "- {:<width$}  {:>size$}  {}\n",
                    r.path.display(),
                    human_bytes(r.bytes),
                    plural(r.files, "file"),
                )
            })
            .collect()
    }
}

/// Uncoloured unified diff of one file, three lines of context. Empty when nothing differs.
pub fn unified_diff(path: &Path, before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(
            &format!("a/{}", path.display()),
            &format!("b/{}", path.display()),
        )
        .to_string()
}

/// Colour diff-shaped text for stdout: green additions, red removals, cyan hunk headers, bold
/// file headers. owo-colors decides per stream (tty, `NO_COLOR`, `CLICOLOR`, `TERM=dumb`), so
/// a piped run stays plain. Lines with no marker pass through.
pub fn paint(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with("+++") || line.starts_with("---") {
                line.if_supports_color(Stream::Stdout, |t| t.bold())
                    .to_string()
            } else if line.starts_with('@') {
                line.if_supports_color(Stream::Stdout, |t| t.cyan())
                    .to_string()
            } else if line.starts_with('+') {
                line.if_supports_color(Stream::Stdout, |t| t.green())
                    .to_string()
            } else if line.starts_with('-') {
                line.if_supports_color(Stream::Stdout, |t| t.red())
                    .to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `1023 B`, `1.0 KB`, `1.5 MB`: one decimal past bytes, binary units.
pub fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n < KB {
        return format!("{} B", n as u64);
    }
    for (unit, div) in [("KB", KB), ("MB", KB * KB), ("GB", KB * KB * KB)] {
        if n < div * KB {
            return format!("{:.1} {unit}", n / div);
        }
    }
    format!("{:.1} TB", n / (KB * KB * KB * KB))
}

/// `1204311` as `1 204 311`: a seven-digit file count is unreadable ungrouped.
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn edit_totals(t: &Totals) -> String {
    let mut parts = vec![format!(
        "{} changed",
        plural(t.files_changed as u64, "file")
    )];
    if t.insertions > 0 {
        parts.push(format!("{}(+)", plural(t.insertions as u64, "insertion")));
    }
    if t.deletions > 0 {
        parts.push(format!("{}(-)", plural(t.deletions as u64, "deletion")));
    }
    parts.join(", ")
}

fn removal_totals(t: &Totals) -> String {
    format!(
        "{}, {}, -{}",
        plural(t.paths_removed as u64, "path"),
        plural(t.files_removed, "file"),
        human_bytes(t.bytes_removed)
    )
}

fn plural(n: u64, noun: &str) -> String {
    format!("{} {noun}{}", group(n), if n == 1 { "" } else { "s" })
}

fn column(widths: impl Iterator<Item = usize>) -> usize {
    widths.max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_has_git_headers_and_is_empty_without_change() {
        let d = unified_diff(Path::new("c.toml"), "a\nb\n", "a\nc\n");
        assert!(d.starts_with("--- a/c.toml\n+++ b/c.toml\n@@ "), "{d}");
        assert!(d.contains("-b\n+c\n"), "{d}");
        assert_eq!(unified_diff(Path::new("c.toml"), "same\n", "same\n"), "");
    }

    #[test]
    fn diff_mode_prints_the_diffs_then_the_totals() {
        let mut p = Preview::default();
        p.edit("a.toml", "x\n", "y\nz\n").edit("same", "s\n", "s\n");
        let out = p.render(Mode::Diff);
        assert!(out.starts_with("--- a/a.toml\n"), "{out}");
        assert!(
            out.ends_with("1 file changed, 2 insertions(+), 1 deletion(-)\n"),
            "{out}"
        );
        assert_eq!(p.edits.len(), 1, "an unchanged file is not an edit");
    }

    #[test]
    fn stat_mode_prints_one_aligned_line_per_edit_and_no_diff() {
        let mut p = Preview::default();
        p.edit("a", "", "1\n2\n3\n").edit("long/name", "1\n", "");
        assert_eq!(
            p.render(Mode::Stat),
            " a         | 3 +++\n long/name | 1 -\n2 files changed, 3 insertions(+), 1 deletion(-)\n"
        );
    }

    #[test]
    fn stat_bars_shrink_past_forty_marks_and_keep_a_mark_for_small_edits() {
        let big: String = (0..400).map(|i| format!("{i}\n")).collect();
        let mut p = Preview::default();
        p.edit("big", "", &big).edit("small", "", "1\n");
        let out = p.render(Mode::Stat);
        assert!(
            out.contains(&format!("| 400 {}\n", "+".repeat(40))),
            "{out}"
        );
        assert!(out.contains(" small |   1 +\n"), "{out}");
    }

    #[test]
    fn removals_print_size_and_file_count_in_both_modes() {
        let mut p = Preview::default();
        p.remove("wt/a/target", 13_000_000_000, 48_213)
            .remove("wt/b/node_modules", 1024, 1);
        let want = "- wt/a/target        12.1 GB  48 213 files\n\
                    - wt/b/node_modules   1.0 KB  1 file\n\
                    2 paths, 48 214 files, -12.1 GB\n";
        assert_eq!(p.render(Mode::Diff), want);
        assert_eq!(p.render(Mode::Stat), want);
    }

    #[test]
    fn an_empty_preview_renders_nothing() {
        let p = Preview::default();
        assert!(p.is_empty());
        assert_eq!(p.render(Mode::Diff), "");
        assert_eq!(p.totals(), Totals::default());
    }

    #[test]
    fn json_carries_counts_and_not_the_diff_text() {
        let mut p = Preview::default();
        p.edit("a", "x\n", "y\n").remove("t", 10, 2);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v["edits"][0],
            serde_json::json!({"path": "a", "insertions": 1, "deletions": 1})
        );
        assert_eq!(
            v["removals"][0],
            serde_json::json!({"path": "t", "bytes": 10, "files": 2})
        );
    }

    #[test]
    fn paint_keeps_unmarked_lines_and_the_text_when_colour_is_off() {
        // Piped test output is not a terminal, so owo-colors leaves the text alone.
        let text = "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n1 file changed";
        assert_eq!(paint(text), text);
    }

    #[test]
    fn human_bytes_uses_binary_units() {
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536 * 1024), "1.5 MB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024 * 1024), "5.0 TB");
    }

    #[test]
    fn group_splits_thousands_with_spaces() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1 000");
        assert_eq!(group(1_204_311), "1 204 311");
    }
}
