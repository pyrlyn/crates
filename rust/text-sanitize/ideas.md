# Ideas

- Move cox's `redact::scrub` (secret patterns) and `truncate` (display width) here under their own modules, so cox-sanitize can disappear.
- ketch's `ui` and `registry push` print through `sanitize_prose`; give them one `Display` wrapper so a caller cannot forget it.
