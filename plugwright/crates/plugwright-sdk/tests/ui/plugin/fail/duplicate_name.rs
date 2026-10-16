use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    fn run(&self) {}

    #[plugwright(name = "run")]
    fn run_again(&self) {}
}

fn main() {}
