# shell-completions

Shared shell completion generation for rtok, ketch and cox. Wraps `clap_complete`
and provides the existing Clink generator extracted from rtok. Supports Bash,
Zsh, Fish, PowerShell, Elvish and Clink without reading application state.

Use `generate(Shell, Command, &mut Write)` for the six-shell interface, or
`script(generator, Command)` to obtain bytes for application-specific augmentation.

The crate is a local path dependency and is not published. Installation, package
lookup and prompts belong to the applications.
