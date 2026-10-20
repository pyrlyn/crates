use plugwright::interface;

#[interface]
trait Calc {
    fn value(&mut self) -> u8;
    fn nothing(&mut self);
}

fn main() {}
