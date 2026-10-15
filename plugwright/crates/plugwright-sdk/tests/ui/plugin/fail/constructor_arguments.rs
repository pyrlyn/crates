use plugwright_sdk::plugin;

struct S;

#[plugin]
impl S {
    fn new(a: u8, b: u8) -> Self {
        let _ = (a, b);
        S
    }
}

fn main() {}
