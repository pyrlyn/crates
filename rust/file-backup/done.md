# file-backup — completed tasks

Extracted from `rtok-agent-sdk`: `<name>.bak-<unix-seconds>`, skip when a sibling already holds the same bytes, suffix `-n` when the clock collides. Used by ketch `config reset` and rtok agent setup.

### T1. Clean up target dirs with dunnage after tests

Added a `justfile` with a `dunnage` recipe (lossless compress + dedupe of the
workspace's cargo `target/` dir; never deletes, keeps mtimes) and wired the
`test` recipe to run it afterward via a `just` post-dependency. Tolerant of
machines without `dunnage` installed.
