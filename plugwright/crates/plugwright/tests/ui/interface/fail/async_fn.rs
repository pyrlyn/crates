use plugwright::interface;

#[interface]
trait Calc {
    async fn run(&mut self) -> plugwright::Result<()>;
}

fn main() {}
