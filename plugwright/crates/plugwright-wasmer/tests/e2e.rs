//! Runs the `wasm-hello` example on wasmer through the plugwright manager.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use plugwright::{ManagerOptions, PluginError, PluginKind, PluginManager, PluginSpec, Source, WasmError};
use plugwright_wasmer::WasmerEngine;

fn text(r: Result<Vec<u8>, PluginError>) -> String {
    String::from_utf8(r.unwrap()).unwrap()
}

fn manager() -> PluginManager {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").canonicalize().unwrap();
    let mut o = ManagerOptions::new(examples);
    o.cache_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugwright-e2e");
    let mut m = PluginManager::new(o).unwrap();
    m.register_engine(Arc::new(WasmerEngine::new()));
    m
}

fn spec(id: &str) -> PluginSpec {
    let mut s = PluginSpec::new(id, PluginKind::Wasm, Source::Path("wasm-hello".into()));
    s.engine = Some("wasmer".into());
    s
}

#[test]
fn wasm_on_wasmer() {
    let mut m = manager();
    let mut s = spec("w");
    s.limits.fuel = Some(5_000_000);
    s.limits.max_memory_bytes = Some(8 << 20);
    s.limits.timeout = Some(Duration::from_millis(500));
    m.load(s).unwrap();
    assert_eq!(m.list().next().unwrap().1.engine.as_deref(), Some("wasmer"));

    assert!(text(m.call("w", "greet", b"w")).starts_with("Hello, w!"));
    assert_eq!(text(m.call("w", "reverse", b"abc")), "cba");
    let looped = m.call("w", "loop", b"").unwrap_err();
    assert!(matches!(looped, PluginError::Wasm(WasmError::OutOfFuel)), "{looped}");
    let oom = m.call("w", "oom", b"").unwrap_err();
    assert!(matches!(oom, PluginError::Wasm(WasmError::MemoryLimit)), "{oom}");
    let trap = m.call("w", "trap", b"").unwrap_err();
    assert!(matches!(trap, PluginError::Wasm(WasmError::Trap(_))), "{trap}");
    assert!(text(m.call("w", "greet", b"again")).contains("call #1"), "fresh instance after trap");
}

/// With practically unlimited fuel, only the wall clock can stop `loop`.
#[test]
#[cfg(unix)]
fn wall_clock_timeout_interrupts_the_guest() {
    let mut m = manager();
    let mut s = spec("t");
    s.limits.fuel = Some(u64::MAX / 2);
    s.limits.timeout = Some(Duration::from_millis(300));
    m.load(s).unwrap();
    let started = std::time::Instant::now();
    let r = m.call("t", "loop", b"");
    let took = started.elapsed();
    assert!(matches!(r, Err(PluginError::Wasm(WasmError::Timeout))), "{r:?}");
    assert!(took < Duration::from_millis(300) + plugwright_wasmer::GRACE, "stopped after {took:?}");
    assert!(text(m.call("t", "greet", b"t")).starts_with("Hello, t!"), "usable after timeout");
}
