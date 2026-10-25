# text-sanitize

Plan for the `text-sanitize` crate of https://github.com/pyrlyn/crates: the shared
terminal and bidi guard for untrusted text.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |
| T3 | todo | P2 | 2 | 0% | |
| T4 | todo | P2 | 3 | 0% | |

### T2. Adopt text-sanitize in the coding agent

Replace `agent-sanitize`'s `sanitize` and `sanitize_with` with `text_sanitize::sanitize` and
`sanitize_with` (the `TERMINAL` preset); `agent-sanitize` keeps `redact` and `truncate`. Needs the
crate published and the creator's permission to touch the coding agent. Done when the coding agent's suite is green on the
crate and its own copy of the scanner is gone.

### T3. Adopt text-sanitize in the token CLI

Replace `characters` and `skip_escape` in `src/sanitize.rs` with `terminal_noise`, `clean` with
`Options { line_ends: LineEnds::Lf, ..Options::RECORD }` and `text_sanitize::skip_escape`; the
wrapper blocks, whitespace folding and JSON walker stay in the token CLI. Needs the crate published and the
creator's permission to touch the token CLI. Done when the token CLI's suite is green on the crate.

### T4. Adopt text-sanitize in the package manager

Replace `changelog::sanitize` with `text_sanitize::sanitize_prose` (the `PROSE` preset), consumed
as a registry version with a gitignored `paths` override for local work, as the package manager does for
`file-backup`. Needs the crate published and the creator's permission to touch the package manager. Done when
The package manager's suite is green on the crate; mind the one change in `PROSE` (U+2028 and U+2029 are now
removed, as the package manager's doc comment already claimed).
