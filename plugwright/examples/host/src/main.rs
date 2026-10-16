//! `plugwright-host` — demo host.
//!
//! ```text
//! plugwright-host [--manifest PATH] demo                      # load everything and misbehave on purpose
//! plugwright-host [--manifest PATH] list                      # load and print plugins
//! plugwright-host [--manifest PATH] call ID METHOD [INPUT]    # one call
//! plugwright-host [--manifest PATH] build                     # build/cache only
//! plugwright-host [--manifest PATH] gc                        # drop unreferenced cache entries
//! ```
//!
//! `RUST_LOG=plugwright=debug` shows build, cache, load and call events.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use plugwright::{PluginError, PluginManager, PluginSpec};
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Must come first: isolated plugins re-exec this binary as their runner.
    plugwright::maybe_run_runner();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .init();

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut manifest = PathBuf::from("examples/plugwright.toml");
    if let Some(i) = args.iter().position(|a| a == "--manifest") {
        if i + 1 >= args.len() {
            eprintln!("--manifest needs a path");
            return ExitCode::from(2);
        }
        manifest = PathBuf::from(args.remove(i + 1));
        args.remove(i);
    }
    let cmd = args.first().map(String::as_str).unwrap_or("demo");
    let res = match cmd {
        "demo" => demo(&manifest),
        "list" => list(&manifest),
        "build" => build(&manifest),
        "gc" => gc(&manifest),
        "call" if args.len() >= 3 => call(&manifest, &args[1], &args[2], args.get(3).map(String::as_str).unwrap_or("")),
        _ => {
            eprintln!("usage: plugwright-host [--manifest PATH] <demo|list|build|gc|call ID METHOD [INPUT]>");
            return ExitCode::from(2);
        }
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn open(manifest: &Path) -> Result<(PluginManager, Vec<PluginSpec>), PluginError> {
    let (mgr, m) = PluginManager::from_manifest(manifest)?;
    Ok((mgr, m.plugins))
}

fn load_all(mgr: &mut PluginManager, specs: Vec<PluginSpec>) {
    for (id, r) in mgr.load_all(specs) {
        if let Err(e) = r {
            println!("  ✗ {id}: {e}");
        }
    }
}

fn list(manifest: &Path) -> Result<(), PluginError> {
    let (mut mgr, specs) = open(manifest)?;
    load_all(&mut mgr, specs);
    print_table(&mgr);
    Ok(())
}

fn print_table(mgr: &PluginManager) {
    println!("\n{:<16} {:<10} {:<10} {:<20} {:<8} sha256", "id", "backend", "engine", "name@version", "cached");
    for (id, info, art) in mgr.list() {
        println!(
            "{:<16} {:<10} {:<10} {:<20} {:<8} {}",
            id,
            info.backend.to_string(),
            info.engine.as_deref().unwrap_or("-"),
            format!("{}@{}", info.name, info.version),
            art.cached,
            &art.sha256[..16]
        );
    }
    println!();
}

fn build(manifest: &Path) -> Result<(), PluginError> {
    let (mut mgr, specs) = open(manifest)?;
    for s in specs.iter().filter(|s| s.enabled) {
        match mgr.prepare(s) {
            Ok(a) => println!("  ✓ {:<16} {} (cached: {})", s.id, a.path.display(), a.cached),
            Err(e) => println!("  ✗ {:<16} {e}", s.id),
        }
    }
    Ok(())
}

fn gc(manifest: &Path) -> Result<(), PluginError> {
    let (mgr, _) = open(manifest)?;
    println!("removed {} cache entries", mgr.gc_cache()?);
    Ok(())
}

fn call(manifest: &Path, id: &str, method: &str, input: &str) -> Result<(), PluginError> {
    let (mut mgr, specs) = open(manifest)?;
    let spec = specs.into_iter().find(|s| s.id == id).ok_or_else(|| PluginError::NotFound(id.into()))?;
    mgr.load(spec)?;
    let out = mgr.call(id, method, input.as_bytes())?;
    println!("{}", String::from_utf8_lossy(&out));
    Ok(())
}

/// Print one call's outcome without stopping the demo.
fn show(mgr: &mut PluginManager, id: &str, method: &str, input: &str) {
    let label = format!("{id}.{method}({input})");
    match mgr.call(id, method, input.as_bytes()) {
        Ok(out) => println!("  ✓ {label:<44} → {}", String::from_utf8_lossy(&out)),
        Err(e) => println!("  ✗ {label:<44} → {e}"),
    }
}

/// The host's view of `examples/native-hello`.
#[plugwright::interface]
trait Hello {
    fn add(&mut self, a: i64, b: i64) -> plugwright::Result<Sum>;
}

#[derive(Deserialize)]
struct Sum {
    sum: i64,
}

fn demo(manifest: &Path) -> Result<(), PluginError> {
    let (mut mgr, specs) = open(manifest)?;
    println!("== loading plugins (engines: {})", mgr.engines().collect::<Vec<_>>().join(", "));
    load_all(&mut mgr, specs.clone());
    print_table(&mgr);

    if mgr.get_mut("hello-native").is_some() {
        println!("== native, in-process");
        show(&mut mgr, "hello-native", "greet", "world");
        if let Ok(bound) = mgr.bind("hello-native") {
            // Typed client generated by `#[plugwright::interface]`; arguments travel as `[2, 40]`.
            match HelloClient::new(bound).add(2, 40) {
                Ok(v) => println!("  ✓ {:<44} → {}", "hello-native.add(2, 40) [typed]", v.sum),
                Err(e) => println!("  ✗ add → {e}"),
            }
        }
        show(&mut mgr, "hello-native", "panic", "");
        show(&mut mgr, "hello-native", "nope", "");
        show(&mut mgr, "hello-native", "greet", "still alive");
    }

    if mgr.get_mut("hello-isolated").is_some() {
        println!("\n== native, isolated in a child process (host pid {})", std::process::id());
        show(&mut mgr, "hello-isolated", "greet", "world");
        show(&mut mgr, "hello-isolated", "print", "");
        show(&mut mgr, "hello-isolated", "panic", "");
        show(&mut mgr, "hello-isolated", "crash", "");
        show(&mut mgr, "hello-isolated", "greet", "after segfault");
        show(&mut mgr, "hello-isolated", "abort", "");
        show(&mut mgr, "hello-isolated", "sleep", "5000");
        show(&mut mgr, "hello-isolated", "greet", "after timeout");
        show(&mut mgr, "hello-isolated", "crash", "");
        show(&mut mgr, "hello-isolated", "greet", "restart budget exhausted?");
    }

    for id in ["hello-wasmtime", "hello-wasmi"] {
        if mgr.get_mut(id).is_none() {
            continue;
        }
        println!("\n== wasm: {id}");
        show(&mut mgr, id, "greet", "world");
        show(&mut mgr, id, "reverse", "plugwright");
        show(&mut mgr, id, "loop", "");
        show(&mut mgr, id, "oom", "");
        show(&mut mgr, id, "trap", "");
        show(&mut mgr, id, "greet", "fresh instance");
    }

    println!("\n== version mismatch is an error, not a crash");
    if let Some(mut spec) = specs.iter().find(|s| s.id == "hello-native").cloned() {
        spec.id = "hello-v2".into();
        spec.version = Some("^2".parse().expect("valid req"));
        match mgr.load(spec) {
            Ok(_) => println!("  ? unexpectedly loaded"),
            Err(e) => println!("  ✗ {e}"),
        }
    }

    println!("\n== reload (cache hit, fresh instance)");
    match mgr.reload("hello-wasmtime") {
        Ok(info) => println!("  ✓ reloaded {}@{}", info.name, info.version),
        Err(e) => println!("  ✗ {e}"),
    }
    show(&mut mgr, "hello-wasmtime", "greet", "after reload");

    println!("\n== unloading");
    let ids: Vec<String> = mgr.list().map(|(id, ..)| id.to_owned()).collect();
    for id in ids {
        let release = mgr.unload(&id)?;
        println!("  ✓ unloaded {id} ({release})");
    }
    if mgr.leaked_images() > 0 {
        println!(
            "  ! {} native image(s) stayed mapped; isolation = \"process\" guarantees unloading",
            mgr.leaked_images()
        );
    }
    println!("\nhost process {} survived everything", std::process::id());
    Ok(())
}
