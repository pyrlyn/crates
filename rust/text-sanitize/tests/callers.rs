//! Each caller's own test suite, ported against the preset that replaces its sanitizer: cox's
//! `cox-sanitize` against `sanitize`, ketch's `changelog::sanitize` against `sanitize_prose`,
//! rtok's `sanitize::characters` against `terminal_noise` and `RECORD` with `LineEnds::Lf`.

use std::borrow::Cow;

use text_sanitize::{
    Escapes, Invisibles, LineEnds, Options, clean, sanitize, sanitize_prose, sanitize_with,
    skip_escape, terminal_noise,
};

const RECORD_LF: Options = Options {
    line_ends: LineEnds::Lf,
    ..Options::RECORD
};

fn record_lf(s: &str) -> String {
    clean(s, &RECORD_LF).into_owned()
}

// cox-sanitize

#[test]
fn cox_keeps_newlines_tabs_and_shaping_joiners() {
    assert_eq!(sanitize("a\tb\nc"), "a\tb\nc");
    assert_eq!(sanitize("👩\u{200d}💻"), "👩\u{200d}💻");
    assert_eq!(sanitize("a\u{200b}\u{200b}b"), "ab");
    assert_eq!(sanitize_with("a\u{200b}\u{200b}b", true), "a∅b");
}

#[test]
fn cox_cuts_an_unterminated_osc_at_the_line_end() {
    assert_eq!(sanitize("\u{1b}]0;title\nnext"), "\nnext");
}

#[test]
fn cox_strips_model_osc8() {
    let link = "\u{1b}]8;;https://evil.test\u{1b}\\click\u{1b}]8;;\u{1b}\\ me";
    assert_eq!(sanitize(link), "click me");
    assert_eq!(sanitize("\u{9d}8;;file:///etc\u{9c}x"), "x");
}

// ketch changelog::sanitize

#[test]
fn ketch_changelog_cannot_drive_the_terminal_it_is_printed_to() {
    let hostile = "\u{1b}[2J\u{1b}]0;pwned\u{7}real\rtext\u{202e}reversed\u{9b}m";
    let body = sanitize_prose(hostile);
    assert_eq!(body, "[2J]0;pwnedrealtextreversedm");
    assert!(!body.contains('\u{1b}'));
    assert!(!body.contains('\r'));
}

#[test]
fn ketch_drops_the_invisible_characters_that_reorder_a_line() {
    let hostile = "a\u{200e}b\u{200f}c\u{61c}d\u{200b}e\u{feff}f\u{2066}g\u{e0041}";
    assert_eq!(sanitize_prose(hostile), "abcdefg");
}

#[test]
fn ketch_keeps_the_shape_of_the_prose() {
    assert_eq!(
        sanitize_prose("## 1.0\n\n\t- a\r\n- b\n"),
        "## 1.0\n\n\t- a\n- b\n"
    );
}

// rtok sanitize::{text (character part), terminal_noise}

#[test]
fn rtok_drops_each_kind_of_character_noise() {
    let cases = [
        ("csi_colour", "\u{1b}[31mred\u{1b}[0m", "red"),
        ("csi_cursor", "a\u{1b}[2K\u{1b}[1Gb", "ab"),
        (
            "osc_link_bel",
            "\u{1b}]8;;https://x\u{7}link\u{1b}]8;;\u{7}",
            "link",
        ),
        (
            "osc_link_st",
            "\u{1b}]8;;https://x\u{1b}\\link\u{1b}]8;;\u{1b}\\",
            "link",
        ),
        ("two_byte_escape", "a\u{1b}=b", "ab"),
        ("escape_before_utf8", "\u{1b}é", "é"),
        ("unterminated_csi", "a\u{1b}[", "a"),
        ("unterminated_csi_over_utf8", "a\u{1b}[ёж", "a"),
        ("control", "a\u{7}b\u{0}c\u{8}d\u{7f}e", "abcde"),
        ("tab_kept", "a\tb", "a\tb"),
        ("crlf", "a\r\nb\r\n", "a\nb\n"),
        ("lone_cr", "a\rb", "a\nb"),
        ("zero_width", "a\u{200b}b\u{2060}c\u{feff}", "abc"),
        ("emoji_joiner_kept", "👩\u{200d}💻", "👩\u{200d}💻"),
        (
            "indent_kept",
            "    fn x() {\n\tlet y;\n}",
            "    fn x() {\n\tlet y;\n}",
        ),
    ];
    for (name, input, want) in cases {
        let got = record_lf(input);
        assert_eq!(got, want, "{name}: {input:?}");
        assert_eq!(
            record_lf(&got),
            got,
            "{name}: cleaning twice changes nothing"
        );
    }
}

#[test]
fn rtok_terminal_noise_touches_nothing_else() {
    let cases = [
        (
            "escapes",
            "\u{1b}[31mred\u{1b}[0m \u{1b}]8;;u\u{7}l\u{1b}]8;;\u{7}",
            "red l",
        ),
        (
            "control_and_zero_width",
            "a\u{7}b\u{0}c\u{200b}d\u{feff}e\u{2060}",
            "abcde",
        ),
        (
            "whitespace_kept",
            "a  \r\n\r\n\n\n\tb \t\n",
            "a  \r\n\r\n\n\n\tb \t\n",
        ),
        (
            "wrapper_kept",
            "<system-reminder>\n\u{1b}[1mx\u{1b}[0m\n</system-reminder>\n\n\n",
            "<system-reminder>\nx\n</system-reminder>\n\n\n",
        ),
        ("emoji_joiner_kept", "👩\u{200d}💻", "👩\u{200d}💻"),
        ("bidi_kept", "a\u{202e}b", "a\u{202e}b"),
    ];
    for (name, input, want) in cases {
        let got = terminal_noise(input);
        assert_eq!(got, want, "{name}: {input:?}");
        assert_eq!(terminal_noise(&got), got, "{name}: idempotent");
    }
}

#[test]
fn rtok_terminal_noise_borrows_clean_text() {
    assert!(matches!(terminal_noise("a  \r\nb\t"), Cow::Borrowed(_)));
}

#[test]
fn rtok_skip_escape_returns_the_index_after_the_sequence() {
    let body = b"a\x1b[31mb\x1b]0;t\x07c\x1b=d";
    assert_eq!(skip_escape(body, 1), 6);
    assert_eq!(skip_escape(body, 7), 13);
    assert_eq!(skip_escape(body, 14), 16);
    // A truncated stream ends the sequence at its end, never past it.
    assert_eq!(skip_escape(b"\x1b", 0), 1);
    assert_eq!(skip_escape(b"\x1b[1;", 0), 4);
    assert_eq!(skip_escape(b"\x1b]0;t", 0), 5);
}

// Behaviour no source test pins: the union of the three.

#[test]
fn clean_text_is_borrowed_by_every_preset() {
    let s = "plain text\n\nwith one blank line and\ttabs, é, 漢字, 👩\u{200d}💻";
    for options in [Options::TERMINAL, Options::PROSE, Options::RECORD] {
        let want_owned = options.invisibles == Invisibles::All;
        // PROSE strips the joiner, the other two keep it.
        assert_eq!(
            matches!(clean(s, &options), Cow::Owned(_)),
            want_owned,
            "{options:?}"
        );
    }
    assert!(matches!(
        clean("plain\ttext\n", &Options::PROSE),
        Cow::Borrowed(_)
    ));
}

#[test]
fn marks_show_where_each_kind_of_text_was_cut() {
    let marked = Options {
        marks: true,
        ..Options::TERMINAL
    };
    let got = clean(
        "a\u{1b}[31mb\u{7}c\u{7f}d\u{85}e\re\u{202e}f\u{200b}\u{200b}g",
        &marked,
    );
    assert_eq!(got, "a␛b␇c␡d␛e␍e⇄f∅g");
}

#[test]
fn c1_sequences_are_removed_whole_when_parsed_and_by_character_otherwise() {
    assert_eq!(sanitize("a\u{9b}31mb"), "ab");
    assert_eq!(sanitize("a\u{90}payload\u{9c}b"), "ab");
    assert_eq!(sanitize_prose("a\u{9b}31mb"), "a31mb");
}

#[test]
fn a_malformed_csi_ends_where_the_next_sequence_starts() {
    assert_eq!(sanitize("\u{1b}[31\u{1b}[0mx"), "x");
    assert_eq!(sanitize("\u{1b}[31\nnext"), "\nnext");
    assert_eq!(sanitize("a\u{1b}\nb"), "a\nb");
    assert_eq!(sanitize("\u{1b}(0x\u{1b}#8y"), "xy");
}

#[test]
fn a_joiner_after_a_kept_joiner_is_part_of_a_run() {
    assert_eq!(sanitize("a\u{200d}\u{1b}[0m\u{200d}b"), "a\u{200d}b");
}

#[test]
fn the_bidi_flag_and_the_invisible_set_are_independent() {
    let s = "a\u{202e}b\u{200b}c";
    let no_bidi = Options {
        bidi: false,
        ..Options::TERMINAL
    };
    assert_eq!(clean(s, &no_bidi), "a\u{202e}bc");
    let none = Options {
        invisibles: Invisibles::Keep,
        ..no_bidi
    };
    assert_eq!(clean(s, &none), "a\u{202e}b\u{200b}c");
    // `All` already includes the bidi controls.
    let all = Options {
        bidi: false,
        ..Options::PROSE
    };
    assert_eq!(clean(s, &all), "abc");
    assert_eq!(sanitize_prose("a\u{2028}b\u{2029}c\u{ad}d"), "abcd");
}

#[test]
fn line_ends_have_three_spellings() {
    let s = "a\r\nb\rc\n";
    let with = |line_ends| {
        clean(
            s,
            &Options {
                line_ends,
                ..Options::RECORD
            },
        )
        .into_owned()
    };
    assert_eq!(with(LineEnds::Keep), s);
    assert_eq!(with(LineEnds::Lf), "a\nb\nc\n");
    assert_eq!(with(LineEnds::Drop), "a\nbc\n");
}

#[test]
fn controls_only_leaves_the_payload_of_a_sequence() {
    let controls = Options {
        escapes: Escapes::ControlsOnly,
        ..Options::TERMINAL
    };
    assert_eq!(clean("\u{1b}]0;t\u{7}x", &controls), "]0;tx");
}

/// A small deterministic generator: hostile fragments in every order, for every preset.
#[test]
fn every_preset_is_idempotent_and_leaves_nothing_it_removes() {
    const PIECES: [&str; 22] = [
        "a",
        " ",
        "\n",
        "\t",
        "\r",
        "\u{1b}",
        "\u{1b}[",
        "\u{1b}]0;",
        "\u{1b}\\",
        "31m",
        "\u{7}",
        "\u{9b}",
        "\u{9d}",
        "\u{9c}",
        "\u{200b}",
        "\u{200d}",
        "\u{202e}",
        "\u{feff}",
        "é",
        "👩",
        "\u{85}",
        "\u{7f}",
    ];
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    for _ in 0..4000 {
        let mut s = String::new();
        for _ in 0..12 {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            s.push_str(PIECES[(seed >> 33) as usize % PIECES.len()]);
        }
        for options in [
            Options::TERMINAL,
            Options::PROSE,
            Options::RECORD,
            RECORD_LF,
        ] {
            let once = clean(&s, &options).into_owned();
            assert_eq!(clean(&once, &options), once, "{options:?}: {s:?}");
            let left = ['\u{1b}', '\u{9b}', '\u{9d}', '\u{7}', '\u{7f}', '\u{85}']
                .into_iter()
                .find(|c| once.contains(*c));
            assert!(
                left.is_none(),
                "{options:?} left {left:?} of {s:?} in {once:?}"
            );
        }
    }
}
