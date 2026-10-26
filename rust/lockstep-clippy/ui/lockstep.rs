// The crate is named `lockstep` (after this file). It implements locks and threads itself, so
// LOCKSTEP_EXPLICIT_CONCURRENCY stays quiet, and it is in the default `allowed_logging_crates`, so
// LOCKSTEP_LOGGING_ONLY does too.

use std::sync::Mutex;

fn main() {
    let lock = Mutex::new(0);
    let _ = std::thread::spawn(|| {});
    *lock.lock().unwrap() += 1;
    println!("lockstep logs however it needs to");
}
