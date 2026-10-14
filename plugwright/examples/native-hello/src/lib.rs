//! Example native plugin. Besides normal methods it can misbehave on purpose
//! (`panic`, `crash`, `abort`, `sleep`, `print`) to show how the host copes,
//! and probe the sandbox (`read_file`, `write_file`, `connect`, `spawn`).

use plugwright_sdk::{log, plugin};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_greeting")]
    greeting: String,
}

fn default_greeting() -> String {
    "Hello".into()
}

#[derive(Serialize)]
struct Sum {
    sum: i64,
}

struct Hello {
    greeting: String,
    greeted: u64,
}

#[plugin]
impl Hello {
    /// The manifest's `config` table, decoded from JSON.
    fn new(config: Config) -> Self {
        log::info!("native-hello initialised with greeting {:?}", config.greeting);
        Hello { greeting: config.greeting, greeted: 0 }
    }

    /// One string argument travels as raw UTF-8.
    fn greet(&mut self, name: &str) -> String {
        self.greeted += 1;
        format!("{}, {name}! (call #{}, pid {})", self.greeting, self.greeted, std::process::id())
    }

    /// Several arguments travel as a JSON array (`[2, 40]`) or object (`{"a":2,"b":40}`).
    fn add(&self, a: i64, b: i64) -> Result<Sum, &'static str> {
        Ok(Sum { sum: a.checked_add(b).ok_or("overflow")? })
    }

    fn panic(&self) {
        panic!("panic requested by the host")
    }

    fn crash(&self) {
        log::warn("about to dereference a null pointer");
        // SAFETY: none — this is the point. Kills the process with SIGSEGV.
        unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
    }

    fn abort(&self) {
        std::process::abort()
    }

    /// One non-string argument travels as JSON: `5000`.
    fn sleep(&self, ms: u64) -> &'static str {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        "woke up"
    }

    fn print(&self) -> &'static str {
        println!("stray stdout output from a plugin");
        "printed"
    }

    // Sandbox probes: each returns a short description or the OS error.
    fn read_file(&self, path: &str) -> std::io::Result<String> {
        Ok(format!("read {} bytes", std::fs::read(path)?.len()))
    }

    fn write_file(&self, path: &str) -> std::io::Result<&'static str> {
        std::fs::write(path, b"written by a plugin")?;
        Ok("wrote")
    }

    fn connect(&self, addr: &str) -> std::io::Result<&'static str> {
        std::net::TcpStream::connect(addr)?;
        Ok("connected")
    }

    fn spawn(&self) -> std::io::Result<String> {
        let out = std::process::Command::new("/bin/sh").arg("-c").arg("echo spawned").output()?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}
