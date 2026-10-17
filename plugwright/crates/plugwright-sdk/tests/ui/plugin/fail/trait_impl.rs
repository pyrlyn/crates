use plugwright_sdk::plugin;

struct S;

#[plugin]
impl Clone for S {
    fn clone(&self) -> Self {
        S
    }
}

fn main() {}
