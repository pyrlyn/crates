use plugwright::interface;

#[interface]
trait Calc {
    type Output;
    const LIMIT: u8;
}

fn main() {}
