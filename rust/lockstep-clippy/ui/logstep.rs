// The crate is named `logstep` (after this file), the other default entry of
// `allowed_logging_crates`: its logging is not checked, its locks still are.

use std::sync::Mutex;

fn main() {
    println!("logstep is a logging facade");
    let _ = Mutex::new(0);
}
