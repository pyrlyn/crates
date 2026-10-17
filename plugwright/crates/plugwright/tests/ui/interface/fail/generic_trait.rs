use plugwright::interface;

#[interface]
trait Calc<T> {
    fn get(&mut self) -> plugwright::Result<T>;
}

fn main() {}
