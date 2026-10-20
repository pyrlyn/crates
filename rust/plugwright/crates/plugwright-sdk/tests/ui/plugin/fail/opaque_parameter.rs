use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    fn by_impl(&self, value: impl Into<String>) {
        let _ = value.into();
    }

    fn by_dyn(&self, value: dyn std::fmt::Debug) {}

    fn by_infer(&self, value: _) {}
}

fn main() {}
