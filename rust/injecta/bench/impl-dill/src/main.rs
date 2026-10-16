//! Binary used only to measure release binary size for this implementation.

fn main() {
    let sum =
        std::hint::black_box(impl_dill::handler_resolver())() + impl_dill::singleton_resolver()();
    println!("{sum}");
}
