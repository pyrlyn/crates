use plugwright_sdk::plugin;

#[derive(Default)]
struct S<T>(T);

#[plugin]
impl<T> S<T> {}

fn main() {}
