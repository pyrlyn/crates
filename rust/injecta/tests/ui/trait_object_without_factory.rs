use std::sync::Arc;

trait Logger {}

injecta::container! {
    struct App {
        singleton Arc<dyn Logger>,
    }
}

fn main() {}
