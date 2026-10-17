use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    #[plugwright(name = "plugwright.describe")]
    fn describe(&self) {}
}

fn main() {}
