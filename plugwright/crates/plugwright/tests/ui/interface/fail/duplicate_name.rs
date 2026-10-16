use plugwright::interface;

#[interface]
trait Calc {
    fn run(&mut self) -> plugwright::Result<()>;

    #[plugwright(name = "run")]
    fn run_again(&mut self) -> plugwright::Result<()>;
}

fn main() {}
