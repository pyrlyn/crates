
#[derive(injecta::Injectable)]
struct Session;

injecta::container! {
    struct App {
        scoped std::sync::Arc<Session>,
    }
}

fn main() {}
