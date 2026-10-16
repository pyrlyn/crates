use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    #[plugwright(init)]
    fn make(&self) -> Self {
        S
    }
}

fn main() {}
