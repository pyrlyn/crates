//! Criterion suite. Groups:
//!
//! - `compare/*`: the shared 7-type graph in every DI crate and by hand
//!   (`transient_graph`, `singleton`, `cold`), same as the divan `resolve` bench.
//! - `cold_start`: building an empty container (no value created yet).
//! - `singleton`: first resolve (lazy init) vs cached `resolve` (clone) vs `get` (borrow).
//! - `transient`: the `Handler` graph with `ThreadSafe` and `SingleThread` storage.
//! - `deep_chain/{10,20,50}`: a chain of transients vs the same code by hand.
//! - `loop`: 1 000 resolves per iteration (throughput).
//! - `concurrent/{resolve,get,by_hand}/{1,2,4,8}`: threads hammering one shared singleton.
//! - `scope`: opening a scope, scoped values, root values seen through a scope.
//! - `override`: `App::new(..)` vs `App::new(..).with(fake)`.

use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use criterion_bench::{
    App, FakeLogger, Request, RequestHandler, RequestId, Session, StApp, chains, config,
};
use impl_injecta::{Database, Handler, Logger};
use injecta::Resolve;

macro_rules! compare {
    ($c:expr, $($name:literal => $krate:ident),* $(,)?) => {{
        let mut group = $c.benchmark_group("compare");
        $(
            let resolve = $krate::handler_resolver();
            group.bench_function(BenchmarkId::new("transient_graph", $name), |b| b.iter(|| black_box(&resolve)()));
            let resolve = $krate::singleton_resolver();
            group.bench_function(BenchmarkId::new("singleton", $name), |b| b.iter(|| black_box(&resolve)()));
            group.bench_function(BenchmarkId::new("cold", $name), |b| b.iter(|| black_box($krate::cold())));
        )*
        group.finish();
    }};
}

fn compare(c: &mut Criterion) {
    compare!(c,
        "baseline" => impl_baseline,
        "injecta" => impl_injecta,
        "nject" => impl_nject,
        "teloc" => impl_teloc,
        "shaku" => impl_shaku,
        "dill" => impl_dill,
    );
}

fn cold_start(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_start");
    group.bench_function("thread_safe/new", |b| {
        b.iter(|| black_box(App::new(black_box(config()))))
    });
    group.bench_function("single_thread/new", |b| {
        b.iter(|| black_box(StApp::new(black_box(config()))))
    });
    group.bench_function("thread_safe/new_and_first_handler", |b| {
        b.iter(|| {
            App::new(black_box(config()))
                .resolve::<Handler>()
                .service
                .cache
                .capacity
        });
    });
    group.bench_function("single_thread/new_and_first_handler", |b| {
        b.iter(|| {
            StApp::new(black_box(config()))
                .resolve::<Handler>()
                .service
                .cache
                .capacity
        });
    });
    group.bench_function("by_hand/new_app", |b| {
        b.iter(|| black_box(impl_baseline::new_app()))
    });
    group.finish();
}

fn singleton(c: &mut Criterion) {
    let mut group = c.benchmark_group("singleton");
    group.bench_function("thread_safe/first_resolve", |b| {
        b.iter_batched(
            || App::new(config()),
            |app| app.resolve::<Arc<Database>>(),
            BatchSize::SmallInput,
        );
    });
    group.bench_function("single_thread/first_resolve", |b| {
        b.iter_batched(
            || StApp::new(config()),
            |app| app.resolve::<Arc<Database>>(),
            BatchSize::SmallInput,
        );
    });
    let app = App::new(config());
    let st = StApp::new(config());
    let _ = (
        app.resolve::<Arc<Database>>(),
        st.resolve::<Arc<Database>>(),
    );
    group.bench_function("thread_safe/cached_resolve", |b| {
        b.iter(|| black_box(&app).resolve::<Arc<Database>>())
    });
    group.bench_function("single_thread/cached_resolve", |b| {
        b.iter(|| black_box(&st).resolve::<Arc<Database>>())
    });
    group.bench_function("thread_safe/cached_get", |b| {
        b.iter(|| black_box(&app).get::<Arc<Database>>().config.pool_size);
    });
    group.bench_function("single_thread/cached_get", |b| {
        b.iter(|| black_box(&st).get::<Arc<Database>>().config.pool_size);
    });
    group.finish();
}

fn transient(c: &mut Criterion) {
    let mut group = c.benchmark_group("transient");
    let app = App::new(config());
    let st = StApp::new(config());
    let _ = (app.resolve::<Handler>(), st.resolve::<Handler>());
    group.bench_function("thread_safe/handler", |b| {
        b.iter(|| black_box(&app).resolve::<Handler>())
    });
    group.bench_function("single_thread/handler", |b| {
        b.iter(|| black_box(&st).resolve::<Handler>())
    });
    let by_hand = impl_baseline::handler_resolver();
    group.bench_function("by_hand/handler_checksum", |b| {
        b.iter(|| black_box(&by_hand)())
    });
    group.bench_function("thread_safe/handler_checksum", |b| {
        b.iter(|| black_box(&app).resolve::<Handler>().checksum());
    });
    group.finish();
}

fn deep_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("deep_chain");
    macro_rules! depth {
        ($m:ident, $d:literal) => {{
            let leaf = chains::$m::L0 { tag: 1 };
            let container = chains::$m::Chain::new(leaf.clone());
            assert_eq!(chains::$m::injecta(&container), chains::$m::by_hand(&leaf));
            group.bench_with_input(BenchmarkId::new("injecta", $d), &container, |b, c| {
                b.iter(|| chains::$m::injecta(black_box(c)));
            });
            group.bench_with_input(BenchmarkId::new("by_hand", $d), &leaf, |b, l| {
                b.iter(|| chains::$m::by_hand(black_box(l)));
            });
        }};
    }
    depth!(chain10, 10);
    depth!(chain20, 20);
    depth!(chain50, 50);
    group.finish();
}

const LOOP: u64 = 1_000;

fn resolve_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop");
    group.throughput(Throughput::Elements(LOOP));
    let app = App::new(config());
    let by_hand = impl_baseline::new_app();
    group.bench_function("injecta/handler_x1000", |b| {
        b.iter(|| {
            (0..LOOP)
                .map(|_| black_box(&app).resolve::<Handler>().checksum())
                .sum::<u64>()
        });
    });
    group.bench_function("by_hand/handler_x1000", |b| {
        b.iter(|| {
            (0..LOOP)
                .map(|_| impl_baseline::resolve_handler(black_box(&by_hand)))
                .sum::<u64>()
        });
    });
    group.bench_function("injecta/singleton_get_x1000", |b| {
        b.iter(|| {
            (0..LOOP)
                .map(|_| u64::from(black_box(&app).get::<Arc<Database>>().config.pool_size))
                .sum::<u64>()
        });
    });
    group.bench_function("injecta/singleton_resolve_x1000", |b| {
        b.iter(|| {
            (0..LOOP)
                .map(|_| u64::from(black_box(&app).resolve::<Arc<Database>>().config.pool_size))
                .sum::<u64>()
        });
    });
    group.finish();
}

/// Operations per thread per measured iteration.
const PER_THREAD: u64 = 10_000;

fn concurrent(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent");
    group.sample_size(20);
    let app = App::new(config());
    let _ = app.resolve::<Handler>();
    let by_hand = impl_baseline::new_app();
    for threads in [1usize, 2, 4, 8] {
        group.throughput(Throughput::Elements(PER_THREAD * threads as u64));
        let resolve = || u64::from(black_box(&app).resolve::<Arc<Database>>().config.pool_size);
        let get = || u64::from(black_box(&app).get::<Arc<Database>>().config.pool_size);
        let hand = || impl_baseline::resolve_singleton(black_box(&by_hand));
        let handler = || black_box(&app).resolve::<Handler>().checksum();
        let hand_handler = || impl_baseline::resolve_handler(black_box(&by_hand));
        let ops: [(&str, &(dyn Fn() -> u64 + Sync)); 5] = [
            ("singleton_resolve", &resolve),
            ("singleton_get", &get),
            ("singleton_by_hand", &hand),
            ("handler_resolve", &handler),
            ("handler_by_hand", &hand_handler),
        ];
        for (name, op) in ops {
            group.bench_function(BenchmarkId::new(name, threads), |b| {
                b.iter_custom(|iters| timed(threads, iters, op));
            });
        }
    }
    group.finish();
}

/// Wall time from releasing `threads` workers (each running `op`
/// `PER_THREAD` times) until the last one finishes, summed over `iters`.
fn timed(threads: usize, iters: u64, op: &(dyn Fn() -> u64 + Sync)) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let start_line = Barrier::new(threads + 1);
        let finish_line = Barrier::new(threads + 1);
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    start_line.wait();
                    let mut acc = 0u64;
                    for _ in 0..PER_THREAD {
                        acc = acc.wrapping_add(op());
                    }
                    black_box(acc);
                    finish_line.wait();
                });
            }
            start_line.wait();
            let start = Instant::now();
            finish_line.wait();
            total += start.elapsed();
        });
    }
    total
}

fn scope(c: &mut Criterion) {
    let mut group = c.benchmark_group("scope");
    let app = App::new(config());
    let _ = app.resolve::<Handler>();
    group.bench_function("open_resolve_scoped_drop", |b| {
        b.iter(|| {
            Request::new(black_box(&app), RequestId(7))
                .resolve::<Arc<Session>>()
                .id
                .0
        });
    });
    group.bench_function("open_resolve_handler_drop", |b| {
        b.iter(|| {
            Request::new(black_box(&app), RequestId(7))
                .resolve::<RequestHandler>()
                .checksum()
        });
    });
    let request = Request::new(&app, RequestId(7));
    let _ = request.resolve::<Arc<Session>>();
    group.bench_function("cached_scoped_resolve", |b| {
        b.iter(|| black_box(&request).resolve::<Arc<Session>>())
    });
    group.bench_function("cached_scoped_get", |b| {
        b.iter(|| black_box(&request).get::<Arc<Session>>().id.0)
    });
    group.bench_function("root_singleton_via_scope", |b| {
        b.iter(|| black_box(&request).resolve::<Arc<Database>>());
    });
    group.bench_function("root_transient_in_scope", |b| {
        b.iter(|| black_box(&request).resolve::<Handler>())
    });
    group.finish();
}

fn overrides(c: &mut Criterion) {
    let mut group = c.benchmark_group("override");
    let fake: Arc<dyn Logger> = Arc::new(FakeLogger);
    group.bench_function("new", |b| {
        b.iter(|| black_box(App::new(black_box(config()))))
    });
    group.bench_function("new_with_fake_logger", |b| {
        b.iter(|| black_box(App::new(black_box(config())).with(Arc::clone(&fake))));
    });
    group.bench_function("new_with_fake_logger_first_handler", |b| {
        b.iter(|| {
            App::new(black_box(config()))
                .with(Arc::clone(&fake))
                .resolve::<Handler>()
                .checksum()
        });
    });
    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_millis(500)).measurement_time(Duration::from_secs(2));
    targets = compare, cold_start, singleton, transient, deep_chain, resolve_loop, concurrent, scope, overrides
);
criterion_main!(benches);
