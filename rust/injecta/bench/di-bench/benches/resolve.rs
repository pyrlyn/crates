//! Resolve cost of the same 7-type graph (1 instance, 3 singletons incl. a
//! trait object, 3 transients) in each DI crate and in hand-written code.
//!
//! - `transient_graph`: resolve `Handler` (builds 3 transients, clones 4 shared values)
//! - `singleton`: resolve the warm `Database` singleton
//! - `cold`: build the container, resolve `Handler` once, drop everything

use std::hint::black_box;

fn main() {
    divan::main();
}

macro_rules! suite {
    ($($name:ident => $krate:ident),* $(,)?) => {$(
        mod $name {
            use super::black_box;

            #[divan::bench]
            fn transient_graph(bencher: divan::Bencher) {
                let resolve = $krate::handler_resolver();
                black_box(resolve());
                bencher.bench_local(|| black_box(&resolve)());
            }

            #[divan::bench]
            fn singleton(bencher: divan::Bencher) {
                let resolve = $krate::singleton_resolver();
                black_box(resolve());
                bencher.bench_local(|| black_box(&resolve)());
            }

            #[divan::bench]
            fn cold() -> u64 {
                $krate::cold()
            }
        }
    )*};
}

suite! {
    baseline => impl_baseline,
    injecta => impl_injecta,
    nject => impl_nject,
    shaku => impl_shaku,
    teloc => impl_teloc,
    dill => impl_dill,
}
