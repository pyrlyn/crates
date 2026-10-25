# Done

### Extract the operation icons

Moved out of the package manager's `src/ui.rs` and the token CLI's `src/ui/style.rs`, where `OPERATION_ICONS`, `icon`, `ICON_WIDTH` and the gutter padding were identical. The shared table is the twelve rows both have; the token CLI's own verbs stay with the token CLI and pass through `icon_in`. Check: `cargo test -p operation-icons`.
