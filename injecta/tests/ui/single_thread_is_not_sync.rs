use std::sync::Arc;

#[derive(injecta::Injectable)]
struct Db;

injecta::container! {
    #[injecta(storage = injecta::SingleThread)]
    struct App {
        singleton Arc<Db>,
    }
}

fn main() {
    let app = App::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            let _ = &app;
        });
    });
}
