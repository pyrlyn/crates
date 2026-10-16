use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    fn consume(self) {}
}

fn main() {}
