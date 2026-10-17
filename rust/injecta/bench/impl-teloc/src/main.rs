//! Binary used only to measure release binary size for this implementation.

fn main() {
    let sum =
        std::hint::black_box(impl_teloc::handler_resolver())() + impl_teloc::singleton_resolver()();
    println!("{sum}");
}
