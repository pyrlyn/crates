use plugwright_sdk::plugin;

struct S;

#[plugin]
impl S {
    fn new(config: &mut Vec<u8>) -> Self {
        config.clear();
        S
    }

    fn fill(&self, out: &mut String) {
        out.clear();
    }
}

fn main() {}
