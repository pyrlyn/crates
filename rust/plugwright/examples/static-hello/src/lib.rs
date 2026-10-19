//! Example native plugin shipped as a static library (`.a` / `.lib`).
//! The host relinks it into a shared library with the platform linker.

/// No constructor: `#[plugin]` falls back to `Default` and ignores the config.
#[derive(Default)]
struct StaticHello;

#[plugwright_sdk::plugin]
impl StaticHello {
    fn greet(&self, name: &str) -> String {
        format!("Hello from a static library, {name}!")
    }
}
