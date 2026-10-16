use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    #[plugwright(rename = "b")]
    fn a(&self) {}
}

fn main() {}
