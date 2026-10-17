use plugwright_sdk::plugin;

struct S;

#[plugin]
impl S {
    async fn new() -> Self {
        S
    }

    async fn run(&self) {}
}

fn main() {}
