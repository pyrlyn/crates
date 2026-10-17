use std::sync::Arc;

#[derive(injecta::Injectable)]
struct Db;

injecta::container! {
    #[injecta(storage = injecta::SingleThread)]
    struct App {
        singleton Arc<Db>,
    }
}

fn require_sync<T: Sync>(_: &T) {}

fn main() {
    require_sync(&App::new());
}
