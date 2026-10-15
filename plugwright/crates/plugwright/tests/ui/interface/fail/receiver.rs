use plugwright::interface;

#[interface]
trait Calc {
    fn shared(&self) -> plugwright::Result<()>;
    fn owned(self) -> plugwright::Result<()>;
    fn none() -> plugwright::Result<()>;
}

fn main() {}
