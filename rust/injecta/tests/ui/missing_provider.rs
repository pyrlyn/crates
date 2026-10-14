use std::sync::Arc;
use injecta::Injectable;

struct Database;

#[derive(Injectable)]
struct Repo {
    db: Arc<Database>,
}

injecta::container! {
    struct App {
        transient Repo,
    }
}

fn main() {}
