use plugwright_sdk::plugin;

struct S;

#[plugin]
impl S {
    fn new() -> Self {
        S
    }

    #[plugwright(init)]
    fn make() -> Self {
        S
    }
}

fn main() {}
