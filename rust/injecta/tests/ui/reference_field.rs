#[derive(injecta::Injectable)]
struct Repo<'a> {
    db: &'a str,
}

fn main() {}
