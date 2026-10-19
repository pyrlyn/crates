use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin(client = Remote)]
impl S {}

fn main() {}
