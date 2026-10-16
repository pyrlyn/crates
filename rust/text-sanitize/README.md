# text-sanitize

Make text somebody else wrote safe to print: removes terminal escape sequences, control
characters, bidi overrides and invisible characters.

An escape sequence in a model reply, a tool result or a changelog moves the cursor, sets the
title or writes the clipboard; a bidi override makes a line read as the reverse of what it says;
a zero-width run hides text. `clean` removes them, and one `Options` value says how much.

| Preset | Function | Parses sequences | `\r` | Invisibles | Bidi |
| --- | --- | --- | --- | --- | --- |
| `Options::TERMINAL` | `sanitize`, `sanitize_with` | yes | dropped | runs, joiners kept | removed |
| `Options::PROSE` | `sanitize_prose` | no, control characters only | dropped | all | removed |
| `Options::RECORD` | `terminal_noise` | yes | kept | zero-width, joiners kept | kept |

`\n` and `\t` always stay. `sanitize_with(s, true)` leaves a glyph where text was cut. `clean`
borrows its input when nothing was removed. `skip_escape` finds the end of an escape sequence in
a byte stream that may not be valid UTF-8.

```rust
use text_sanitize::{clean, sanitize, LineEnds, Options};

assert_eq!(sanitize("\u{1b}]0;pwned\u{7}ok\u{202e}!"), "ok!");
let lf = Options { line_ends: LineEnds::Lf, ..Options::RECORD };
assert_eq!(clean("a\r\nb\rc", &lf), "a\nb\nc");
```
