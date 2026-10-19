//! Example Wasm plugin (build for `wasm32-unknown-unknown`). No WASI: it can
//! only compute, log and call the functions its host registered. `loop`, `oom` and `trap` exercise the host's limits.

use plugwright_sdk::{log, plugin, serde::Deserialize};

/// Uses the SDK's serde re-export, so the crate needs no serde dependency of its own.
#[derive(Deserialize)]
#[serde(crate = "plugwright_sdk::serde")]
struct Config {
    #[serde(default = "default_greeting")]
    greeting: String,
}

fn default_greeting() -> String {
    "Hello".into()
}

struct Hello {
    greeting: String,
    greeted: u64,
}

#[plugin]
impl Hello {
    fn new(config: Config) -> Self {
        log::info!("wasm-hello initialised with greeting {:?}", config.greeting);
        Hello { greeting: config.greeting, greeted: 0 }
    }

    fn greet(&mut self, name: &str) -> String {
        self.greeted += 1;
        format!("{}, {name}! (call #{} inside wasm)", self.greeting, self.greeted)
    }

    fn reverse(&self, s: &str) -> String {
        s.chars().rev().collect()
    }

    /// The host function `name` on `input`, as text.
    fn ask(&self, name: String, input: String) -> Result<String, String> {
        plugwright_sdk::host::call(&name, input.as_bytes()).map(|out| String::from_utf8_lossy(&out).into_owned())
    }

    #[plugwright(name = "loop")]
    fn spin(&self) {
        log::warn("entering an infinite loop");
        let mut x: u64 = 0;
        loop {
            x = std::hint::black_box(x.wrapping_add(1));
        }
    }

    fn oom(&self) {
        log::warn("allocating until the host says no");
        // Reserve without touching: exercises memory.grow, not fuel.
        let mut hog: Vec<Vec<u8>> = Vec::new();
        loop {
            hog.push(Vec::with_capacity(1 << 20));
            std::hint::black_box(&hog);
        }
    }

    fn trap(&self) {
        panic!("explicit trap requested")
    }
}
