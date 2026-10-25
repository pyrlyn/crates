# text-sanitize — completed tasks

### T1. Extract the terminal and bidi guard from the coding agent, the package manager and the token CLI

Merged the coding agent's `agent-sanitize::{sanitize, sanitize_with}`, the package manager's `changelog::sanitize` and the
character half of the token CLI's `sanitize` (`characters`, `terminal_noise`, `skip_escape`) into one scanner,
`clean(s, &Options)`, with one preset per caller: `Options::TERMINAL` (the coding agent), `Options::PROSE`
(the package manager), `Options::RECORD` (the token CLI; `LineEnds::Lf` for its `text`). `skip_escape` is public for
The token CLI's byte scanner. No dependencies. Adoption by the three apps is T2 to T4 in `plan.md`.

Out of scope, and staying where it is: the coding agent's `redact::scrub` (secrets) and `truncate` (display
width), and the token CLI's wrapper-block regex, whitespace folding and JSON walker, which clean a record
and do not guard a terminal. Their tests were not ported.

What each implementation handled (sources: `apps/agent/crates/agent-sanitize/src/lib.rs`,
`apps/pkgmgr/crates/pkgmgr-core/src/changelog.rs`, `apps/tokcli/src/sanitize.rs`, as of this commit):

| Input | the coding agent | the package manager | the token CLI | text-sanitize |
| --- | --- | --- | --- | --- |
| `ESC [ … final` (CSI) | removed whole | `ESC` only, `[2J` stays | removed whole | whole (`Sequences`), `ESC` only (`ControlsOnly`) |
| `ESC ]` OSC, to BEL or ST | removed whole | `ESC`, BEL only | removed whole | same split |
| `ESC P X ^ _` (DCS, SOS, PM, APC) | removed whole | `ESC` only | `ESC` and one byte, payload stays | same split |
| `ESC ( ) * + # %` and a final byte | three characters | `ESC` only | `ESC` and one byte | `ESC`, intermediates `0x20..0x2f`, final `0x30..0x7e` |
| `ESC` then non-ASCII or a newline | eats that character | `ESC` only | eats a newline, keeps non-ASCII | `ESC` only |
| Unterminated sequence | stops before a newline | n/a | runs to the end | stops before a newline; a CSI also before any control byte |
| C1 introducers U+009B, 0090, 0098, 009D, 009E, 009F | removed whole | character only | character only | whole (`Sequences`), character only (`ControlsOnly`) |
| Other C1 (U+0080..U+009F), DEL, C0 except `\n` `\t` `\r` | removed | removed | removed | removed |
| `\r` | removed | removed | kept, or to `\n` in `text` | `LineEnds::{Drop, Keep, Lf}` |
| Bidi U+202A..U+202E, U+2066..U+2069 | removed | removed | kept | `Options::bidi` |
| LRM, RLM (U+200E, U+200F), ALM (U+061C), U+2061..U+2065, U+206A..U+206F, soft hyphen, tag block U+E0000..U+E007F | kept | removed | kept | `Invisibles::All` |
| U+2028, U+2029 | kept | kept (its doc says removed) | kept | `Invisibles::All` |
| ZWSP U+200B, WJ U+2060, BOM U+FEFF | a run is one removal | removed | each removed | `ZeroWidthRuns`, `ZeroWidth`, `All` |
| ZWJ U+200D, ZWNJ U+200C | lone, between visible characters, kept | removed | kept | `ZeroWidthRuns`, `ZeroWidth`, `All` |
| Marks where text was cut | yes | no | no | `Options::marks` |
| Borrows clean input | no | no | yes | `clean`, `terminal_noise` |

Behaviour that changed for a caller, all on malformed or rare input no source test covers:

- `Sequences` removes a sequence's C1 forms whole for the token CLI (it dropped only the introducer) and
  removes DCS, SOS, PM and APC payloads for the token CLI (it kept them).
- An `ESC` before a newline or a non-ASCII character no longer eats it (the coding agent ate both; the token CLI ate
  the newline).
- An unterminated sequence stops before the newline for the token CLI (it ran to the end of the text).
- A CSI ends before any control byte, so `ESC [ 3 1 ESC [ 0 m` is two sequences; the coding agent and the token CLI
  left `0m` behind.
- A ZWJ or ZWNJ is kept only if it is between two visible characters after the cleaning, so
  `a ZWJ ESC[0m ZWJ b` no longer keeps both joiners (the coding agent kept both, and a second pass changed the
  text).
- `PROSE` removes U+2028 and U+2029, which the package manager's doc comment already claimed.

Tests: `tests/callers.rs` holds the coding agent's three `sanitize` tests (its `truncate` test stays with
`truncate`), the package manager's three `sanitize` tests (the hostile-changelog one calls `sanitize_prose` instead
of `from_release`), the token CLI's character cases of `text_drops_each_kind_of_noise` and
`terminal_noise_touches_nothing_else`, `terminal_noise_borrows_clean_text` and a `skip_escape` test,
plus tests for each row above that no source pinned and a seeded 4000-string idempotence check over
every preset. 19 tests; `cargo clippy -p text-sanitize --all-targets -- -D warnings` and
`cargo fmt --all --check` green, tests also green on Rust 1.86.
