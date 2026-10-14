//! Performance tests for the load and call paths, with budgets.
//!
//! ```text
//! cargo bench -p plugwright-host --bench perf             # every case
//! cargo bench -p plugwright-host --bench perf -- fresh/   # cases whose name contains `fresh/`
//! ```
//!
//! Two kinds of case:
//!
//! * hot: repeated in this process (calls, cache hits, loads of a module that
//!   is already compiled); reported per iteration;
//! * fresh: one load at the start of a new process, the way a host starts up
//!   with a warm cache; timed inside the child, so spawning is not counted.
//!
//! Each case reports median, min and max, and the run fails when a median is
//! over its budget. Budgets are about ten times the medians measured on an
//! M-series Mac, tight enough to catch a lost cache or a blocking drop;
//! `PLUGWRIGHT_PERF_SCALE=3` multiplies them on slow or busy machines. Medians are kept in `target/tmp/perf-last.json` and the next run
//! prints the change against them.
//!
//! Without `--bench` (as under `cargo test --benches`) every case runs once
//! as a smoke test: no budgets, nothing recorded.

use std::any::Any;
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use plugwright::{
    Isolation, ManagerOptions, NativeLoader, Plugin, PluginKind, PluginManager, PluginSpec, Source, WasmLoader,
    WasmiEngine,
};

/// Set in a child process: the name of the fresh case it runs.
const CHILD: &str = "PLUGWRIGHT_PERF_CHILD";
/// Set in a child process: the file or crate the case loads.
const INPUT: &str = "PLUGWRIGHT_PERF_INPUT";
const SCALE: &str = "PLUGWRIGHT_PERF_SCALE";
/// A hot sample runs this long, so timer resolution does not matter.
const SAMPLE: Duration = Duration::from_millis(5);
const HOT_SAMPLES: usize = 25;
const FRESH_SAMPLES: usize = 10;

struct Case {
    name: &'static str,
    budget: Duration,
    kind: Kind,
}

enum Kind {
    /// Set up once, then the returned closure is one iteration.
    Hot(fn(&Fixture) -> Box<dyn FnMut()>),
    /// `run` on the input `input` picks, timed inside a fresh process.
    Fresh { input: fn(&Fixture) -> &Path, run: fn(&Path) -> Box<dyn Any> },
}

/// Plugins built before anything is measured.
struct Fixture {
    native: PathBuf,
    wasm: PathBuf,
    native_crate: PathBuf,
    static_crate: PathBuf,
    wasm_crate: PathBuf,
}

fn cases() -> Vec<Case> {
    use Kind::{Fresh, Hot};
    let us = Duration::from_micros;
    let ms = Duration::from_millis;
    vec![
        Case {
            name: "call/native",
            budget: Duration::from_nanos(2_000),
            kind: Hot(|f| {
                let mut p = NativeLoader::new(cache_dir()).unwrap().open(&f.native).unwrap();
                Box::new(move || drop(black_box(p.call("greet", b"x").unwrap())))
            }),
        },
        Case {
            name: "call/native-isolated",
            budget: us(200),
            kind: Hot(|_| {
                let mut m = manager();
                m.load(native_spec(Isolation::Process)).unwrap();
                Box::new(move || drop(black_box(m.call("n", "greet", b"x").unwrap())))
            }),
        },
        Case {
            name: "call/wasm/wasmtime",
            budget: us(20),
            kind: Hot(|f| {
                let mut p = WasmLoader::new(cache_dir()).unwrap().open(&f.wasm).unwrap();
                Box::new(move || drop(black_box(p.call("greet", b"x").unwrap())))
            }),
        },
        Case {
            name: "call/wasm/wasmi",
            budget: us(30),
            kind: Hot(|f| {
                let mut p = wasmi_loader().open(&f.wasm).unwrap();
                Box::new(move || drop(black_box(p.call("greet", b"x").unwrap())))
            }),
        },
        Case {
            name: "build/cache-hit",
            budget: ms(15),
            kind: Hot(|f| {
                let (loader, dir) = (NativeLoader::new(cache_dir()).unwrap(), f.native_crate.clone());
                Box::new(move || drop(black_box(loader.build(&dir).unwrap())))
            }),
        },
        Case {
            name: "open/wasm/wasmtime/module-in-use",
            budget: ms(2),
            kind: Hot(|f| {
                let loader = WasmLoader::new(cache_dir()).unwrap();
                // A live plugin keeps the compiled module shared.
                let (keep, path) = (loader.open(&f.wasm).unwrap(), f.wasm.clone());
                Box::new(move || {
                    let _ = &keep;
                    drop(black_box(loader.open(&path).unwrap()));
                })
            }),
        },
        Case {
            name: "open/wasm/wasmi/module-in-use",
            budget: ms(2),
            kind: Hot(|f| {
                let loader = wasmi_loader();
                let (keep, path) = (loader.open(&f.wasm).unwrap(), f.wasm.clone());
                Box::new(move || {
                    let _ = &keep;
                    drop(black_box(loader.open(&path).unwrap()));
                })
            }),
        },
        Case {
            // A new loader each time: engine, compiled code from disk, an
            // instance, then all of it dropped, epoch thread included.
            name: "open/wasm/wasmtime/new-loader",
            budget: ms(3),
            kind: Hot(|f| {
                let path = f.wasm.clone();
                Box::new(move || drop(black_box(WasmLoader::new(cache_dir()).unwrap().open(&path).unwrap())))
            }),
        },
        Case {
            name: "fresh/open/native",
            budget: ms(15),
            kind: Fresh {
                input: |f| &f.native,
                run: |p| Box::new(NativeLoader::new(cache_dir()).unwrap().open(p).unwrap()),
            },
        },
        Case {
            name: "fresh/open/wasm/wasmtime",
            budget: ms(8),
            kind: Fresh {
                input: |f| &f.wasm,
                run: |p| Box::new(WasmLoader::new(cache_dir()).unwrap().open(p).unwrap()),
            },
        },
        Case {
            name: "fresh/open/wasm/wasmi",
            budget: ms(25),
            kind: Fresh { input: |f| &f.wasm, run: |p| Box::new(wasmi_loader().open(p).unwrap()) },
        },
        Case {
            name: "fresh/load/native",
            budget: ms(30),
            kind: Fresh {
                input: |f| &f.native_crate,
                run: |p| Box::new(NativeLoader::new(cache_dir()).unwrap().load(p).unwrap()),
            },
        },
        Case {
            name: "fresh/load/static",
            budget: ms(30),
            kind: Fresh {
                input: |f| &f.static_crate,
                run: |p| Box::new(NativeLoader::new(cache_dir()).unwrap().load(p).unwrap()),
            },
        },
        Case {
            name: "fresh/load/wasm",
            budget: ms(30),
            kind: Fresh {
                input: |f| &f.wasm_crate,
                run: |p| Box::new(WasmLoader::new(cache_dir()).unwrap().load(p).unwrap()),
            },
        },
        Case {
            name: "fresh/manager/native",
            budget: ms(40),
            kind: Fresh {
                input: |f| &f.native_crate,
                run: |_| {
                    let mut m = manager();
                    m.load(native_spec(Isolation::InProcess)).unwrap();
                    Box::new(m)
                },
            },
        },
        Case {
            name: "fresh/manager/wasm",
            budget: ms(40),
            kind: Fresh {
                input: |f| &f.wasm_crate,
                run: |_| {
                    let mut m = manager();
                    let mut s = PluginSpec::new("w", PluginKind::Wasm, Source::Path("wasm-hello".into()));
                    s.engine = Some("wasmtime".into());
                    m.load(s).unwrap();
                    Box::new(m)
                },
            },
        },
    ]
}

fn main() {
    // Isolated cases re-execute this binary as their runner.
    plugwright::maybe_run_runner();
    if let Ok(name) = std::env::var(CHILD) {
        return child(&name);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let full = args.iter().any(|a| a == "--bench");
    let filter = args.iter().find(|a| !a.starts_with('-')).cloned().unwrap_or_default();
    let scale: f64 = std::env::var(SCALE).ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);

    let fixture = fixture();
    let last_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("perf-last.json");
    let mut last: BTreeMap<String, u64> =
        std::fs::read(&last_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();

    println!("{:<36} {:>10} {:>10} {:>10} {:>10} {:>8}", "case", "median", "min", "max", "budget", "vs last");
    let mut over = Vec::new();
    for case in cases().into_iter().filter(|c| c.name.contains(&filter)) {
        let mut samples = match &case.kind {
            Kind::Hot(setup) => hot(setup(&fixture), full),
            Kind::Fresh { input, .. } => fresh(case.name, input(&fixture), full),
        };
        samples.sort();
        let median = samples[samples.len() / 2];
        let budget = case.budget.mul_f64(scale);
        let change = last
            .get(case.name)
            .filter(|_| full)
            .map(|&was| format!("{:+.0}%", (median.as_nanos() as f64 / was.max(1) as f64 - 1.0) * 100.0))
            .unwrap_or_default();
        let flag = if full && median > budget { "  OVER BUDGET" } else { "" };
        println!(
            "{:<36} {:>10} {:>10} {:>10} {:>10} {:>8}{flag}",
            case.name,
            human(median),
            human(samples[0]),
            human(samples[samples.len() - 1]),
            human(budget),
            change,
        );
        if full {
            last.insert(case.name.to_owned(), median.as_nanos() as u64);
            if median > budget {
                over.push(case.name);
            }
        }
    }
    if full {
        std::fs::write(&last_path, serde_json::to_vec_pretty(&last).unwrap()).unwrap();
    }
    if !over.is_empty() {
        eprintln!("over budget: {}", over.join(", "));
        std::process::exit(1);
    }
}

/// Per-iteration times, one per sample.
fn hot(mut iteration: Box<dyn FnMut()>, full: bool) -> Vec<Duration> {
    let started = Instant::now();
    iteration();
    if !full {
        return vec![started.elapsed()];
    }
    // Warm-up, then size samples from the second (warm) iteration.
    let one = time(&mut iteration, 1);
    let iters = (SAMPLE.as_nanos() / one.as_nanos().max(1)).clamp(1, 1_000_000) as u32;
    (0..HOT_SAMPLES).map(|_| time(&mut iteration, iters) / iters).collect()
}

fn time(iteration: &mut Box<dyn FnMut()>, iters: u32) -> Duration {
    let started = Instant::now();
    for _ in 0..iters {
        iteration();
    }
    started.elapsed()
}

/// Times reported by fresh child processes, after one discarded warm-up run.
fn fresh(name: &str, input: &Path, full: bool) -> Vec<Duration> {
    let runs = if full { FRESH_SAMPLES + 1 } else { 1 };
    let mut samples: Vec<Duration> = (0..runs).map(|_| spawn(name, input)).collect();
    if full {
        samples.remove(0);
    }
    samples
}

fn spawn(name: &str, input: &Path) -> Duration {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe).env(CHILD, name).env(INPUT, input).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{name}: {}\n{stdout}{}", out.status, String::from_utf8_lossy(&out.stderr));
    let ns = stdout.lines().last().and_then(|l| l.strip_prefix("ns=")).and_then(|n| n.parse().ok());
    Duration::from_nanos(ns.unwrap_or_else(|| panic!("{name}: no timing in {stdout:?}")))
}

/// Run one fresh case and print how long it took.
fn child(name: &str) {
    let case = cases().into_iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no case `{name}`"));
    let Kind::Fresh { run, .. } = case.kind else { panic!("`{name}` is not a fresh case") };
    let input = PathBuf::from(std::env::var_os(INPUT).unwrap());
    let started = Instant::now();
    let loaded = run(&input);
    println!("ns={}", started.elapsed().as_nanos());
    // Unloading is not part of the measurement; the process ends here.
    std::mem::forget(loaded);
    std::process::exit(0);
}

fn fixture() -> Fixture {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap();
    let started = Instant::now();
    let native_loader = NativeLoader::new(cache_dir()).unwrap();
    let f = Fixture {
        native: native_loader.build(examples.join("native-hello")).unwrap().path,
        wasm: WasmLoader::new(cache_dir()).unwrap().build(examples.join("wasm-hello")).unwrap().path,
        native_crate: examples.join("native-hello"),
        static_crate: examples.join("static-hello"),
        wasm_crate: examples.join("wasm-hello"),
    };
    native_loader.build(&f.static_crate).unwrap();
    println!("plugins ready in {}", human(started.elapsed()));
    f
}

fn cache_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e")
}

fn wasmi_loader() -> WasmLoader {
    WasmLoader::new(cache_dir()).unwrap().with_engine(Arc::new(WasmiEngine::new()))
}

fn manager() -> PluginManager {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize().unwrap();
    let mut o = ManagerOptions::new(examples);
    o.cache_dir = cache_dir();
    PluginManager::new(o).unwrap()
}

fn native_spec(isolation: Isolation) -> PluginSpec {
    let mut s = PluginSpec::new("n", PluginKind::Native, Source::Path("native-hello".into()));
    s.isolation = Some(isolation);
    s
}

fn human(d: Duration) -> String {
    let ns = d.as_nanos() as f64;
    match ns {
        n if n < 1e3 => format!("{n:.0} ns"),
        n if n < 1e6 => format!("{:.1} µs", n / 1e3),
        n if n < 1e9 => format!("{:.1} ms", n / 1e6),
        n => format!("{:.2} s", n / 1e9),
    }
}
