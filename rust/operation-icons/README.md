# operation-icons

Icons for the verb of a status line, and the gutter that keeps lines with and
without an icon aligned.

- `icon("installing")` is `Some("📦")`; a verb no row names is `None`, so the
  caller falls back to the icon of its own line kind.
- `icon_in(rows, verb)` does the same over a caller's own rows.
- `gutter(icon)` is the icon, padding to `ICON_WIDTH` columns (measured with
  `unicode-width`) and one space; `gutter(None)` is that many blanks.

The crate knows nothing about tones or colours.
