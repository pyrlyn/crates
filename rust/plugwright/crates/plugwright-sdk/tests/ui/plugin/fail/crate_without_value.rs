use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin(crate)]
impl S {}

fn main() {}
