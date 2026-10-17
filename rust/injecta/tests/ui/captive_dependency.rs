use std::sync::Arc;
use injecta::Injectable;

#[derive(Injectable)]
struct Session;

#[derive(Injectable)]
struct Cache {
    session: Arc<Session>,
}

injecta::container! {
    struct App {
        singleton Arc<Cache>,
    }
    scope Request {
        scoped Arc<Session>,
    }
}

fn main() {}
