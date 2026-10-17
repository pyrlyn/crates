use plugwright::interface;

#[interface]
trait Calc {
    fn run<T: Clone>(&mut self, value: T) -> plugwright::Result<()>;
}

fn main() {}
