use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    fn run<T: std::fmt::Debug>(&self, value: T) {
        let _ = value;
    }
}

fn main() {}
