use plugwright::interface;

#[interface]
trait Calc {
    fn fill(&mut self, out: &mut String) -> plugwright::Result<()>;
    fn by_impl(&mut self, value: impl Into<String>) -> plugwright::Result<()>;
}

fn main() {}
