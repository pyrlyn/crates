# Done

### Extract the operation icons

Moved out of ketch's `src/ui.rs` and rtok's `src/ui/style.rs`, where `OPERATION_ICONS`, `icon`, `ICON_WIDTH` and the gutter padding were identical. The shared table is the twelve rows both have; rtok's own verbs stay with rtok and pass through `icon_in`. Check: `cargo test -p operation-icons`.
