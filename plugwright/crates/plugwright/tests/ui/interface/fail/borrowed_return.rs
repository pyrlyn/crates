use plugwright::interface;

#[interface]
trait Calc {
    fn name(&mut self) -> plugwright::Result<&str>;
}

fn main() {}
