//! Binary used only to measure release binary size for this implementation.

fn main() {
    let sum = std::hint::black_box(impl_baseline::handler_resolver())()
        + impl_baseline::singleton_resolver()();
    println!("{sum}");
}
