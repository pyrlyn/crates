use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    fn sum(&self, (a, b): (u8, u8), c: u8) -> u8 {
        a + b + c
    }
}

fn main() {}
