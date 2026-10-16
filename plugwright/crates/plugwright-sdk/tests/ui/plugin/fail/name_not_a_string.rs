use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    #[plugwright(name = b)]
    fn a(&self) {}
}

fn main() {}
