use plugwright_sdk::plugin;

#[derive(Default)]
struct S;

#[plugin]
impl S {
    unsafe extern "C" fn run(&self, n: u8, _: ...) {
        let _ = n;
    }
}

fn main() {}
