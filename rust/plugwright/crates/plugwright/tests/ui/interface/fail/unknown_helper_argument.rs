use plugwright::interface;

#[interface]
trait Calc {
    #[plugwright(skip)]
    fn get(&mut self) -> plugwright::Result<u8>;
}

fn main() {}
