# text-sanitize

Plan for the `text-sanitize` crate of https://github.com/pyrlyn/crates: the shared
terminal and bidi guard for untrusted text.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |
| T3 | todo | P2 | 2 | 0% | |
| T4 | todo | P2 | 3 | 0% | |

### T2. Adopt text-sanitize in cox

Replace `cox-sanitize`'s `sanitize` and `sanitize_with` with `text_sanitize::sanitize` and
`sanitize_with` (the `TERMINAL` preset); `cox-sanitize` keeps `redact` and `truncate`. Needs the
crate published and the creator's permission to touch cox. Done when cox's suite is green on the
crate and its own copy of the scanner is gone.

### T3. Adopt text-sanitize in rtok

Replace `characters` and `skip_escape` in `src/sanitize.rs` with `terminal_noise`, `clean` with
`Options { line_ends: LineEnds::Lf, ..Options::RECORD }` and `text_sanitize::skip_escape`; the
wrapper blocks, whitespace folding and JSON walker stay in rtok. Needs the crate published and the
creator's permission to touch rtok. Done when rtok's suite is green on the crate.

### T4. Adopt text-sanitize in ketch

Replace `changelog::sanitize` with `text_sanitize::sanitize_prose` (the `PROSE` preset), consumed
as a registry version with a gitignored `paths` override for local work, as ketch does for
`file-backup`. Needs the crate published and the creator's permission to touch ketch. Done when
ketch's suite is green on the crate; mind the one change in `PROSE` (U+2028 and U+2029 are now
removed, as ketch's doc comment already claimed).
