//! `#[plugin]` expansion, driven through the `Plugin` trait without any loader.

use plugwright_sdk::{Plugin, plugin, serde::Deserialize};

#[derive(Deserialize)]
#[serde(crate = "plugwright_sdk::serde")]
struct Config {
    start: i64,
}

struct Counter {
    value: i64,
}

#[plugin(no_export)]
impl Counter {
    fn new(config: &Config) -> Result<Self, String> {
        if config.start < 0 { Err("start must be >= 0".into()) } else { Ok(Counter { value: config.start }) }
    }

    fn get(&self) -> i64 {
        self.helper();
        self.value
    }

    fn bump(&mut self, by: Option<i64>) {
        self.value += by.unwrap_or(1);
    }

    fn echo(&self, raw: &[u8]) -> Vec<u8> {
        raw.to_vec()
    }

    fn label(&self, prefix: String, upper: bool) -> String {
        let s = format!("{prefix}{}", self.value);
        if upper { s.to_uppercase() } else { s }
    }

    #[plugwright(name = "check")]
    fn check_positive(&self, n: &i64) -> Result<&'static str, std::num::TryFromIntError> {
        u32::try_from(*n).map(|_| "ok")
    }

    #[plugwright(skip)]
    fn helper(&self) {}

    fn r#type(&self) -> &str {
        "counter"
    }

    fn not_exported() -> i32 {
        7
    }
}

#[derive(Default)]
struct Empty;

#[plugin(no_export)]
impl Empty {}

fn call(p: &mut impl Plugin, method: &str, input: &[u8]) -> Result<String, String> {
    p.call(method, input).map(|v| String::from_utf8(v).unwrap())
}

#[test]
fn constructor_decodes_config_and_reports_errors() {
    assert!(<Counter as Plugin>::new(br#"{"start":5}"#).is_ok());
    assert_eq!(<Counter as Plugin>::new(br#"{"start":-1}"#).err().unwrap(), "start must be >= 0");
    let err = <Counter as Plugin>::new(b"{}").err().unwrap();
    assert!(err.contains("cannot decode `config`"), "{err}");
    assert!(<Empty as Plugin>::new(b"anything").is_ok(), "Default ignores the config");
}

#[test]
fn methods_follow_the_wire_convention() {
    let mut c = <Counter as Plugin>::new(br#"{"start":1}"#).unwrap();
    assert_eq!(call(&mut c, "get", b"ignored").unwrap(), "1");
    assert_eq!(call(&mut c, "bump", b"null").unwrap(), "");
    assert_eq!(call(&mut c, "bump", b"10").unwrap(), "");
    assert_eq!(call(&mut c, "get", b"").unwrap(), "12");
    assert_eq!(c.call("echo", &[0, 255]).unwrap(), [0, 255]);
    assert_eq!(call(&mut c, "label", br#"["n=", true]"#).unwrap(), "N=12");
    assert_eq!(call(&mut c, "label", br#"{"upper":false,"prefix":"n="}"#).unwrap(), "n=12");
    assert_eq!(call(&mut c, "check", b"3").unwrap(), "ok");
    // std's wording changed in Rust 1.99; what matters is that the error's `Display` comes through.
    let std_err = u32::try_from(-3i64).unwrap_err().to_string();
    assert!(call(&mut c, "check", b"-3").unwrap_err().contains(&std_err));
    assert_eq!(call(&mut c, "type", b"").unwrap(), "counter");
    assert_eq!(Counter::not_exported(), 7);
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let mut c = <Counter as Plugin>::new(br#"{"start":0}"#).unwrap();
    for (method, input) in [
        ("bump", &b"\"x\""[..]),
        ("label", b"[1, true]"),
        ("label", b"[\"a\"]"),
        ("label", b"[\"a\", true, 3]"),
        ("label", br#"{"prefix":"a","upper":true,"extra":1}"#),
        ("label", b"42"),
        ("helper", b""),
        ("not_exported", b""),
    ] {
        assert!(call(&mut c, method, input).is_err(), "{method}({})", String::from_utf8_lossy(input));
    }
    let err = call(&mut c, "nope", b"").unwrap_err();
    assert!(err.starts_with("unknown method `nope` (available: get, bump, echo, label, check, type)"), "{err}");
}

#[test]
fn describe_lists_exported_methods() {
    let mut c = <Counter as Plugin>::new(br#"{"start":0}"#).unwrap();
    let doc = call(&mut c, "plugwright.describe", b"").unwrap();
    assert!(doc.starts_with(r#"{"methods":[{"name":"get","params":[]},{"name":"bump","params":["by"]}"#), "{doc}");
    assert!(doc.contains(r#"{"name":"label","params":["prefix","upper"]}"#));
    assert!(!doc.contains("helper") && !doc.contains("not_exported"));
    let mut e = Empty;
    assert_eq!(call(&mut e, "plugwright.describe", b"").unwrap(), r#"{"methods":[]}"#);
    assert_eq!(call(&mut e, "x", b"").unwrap_err(), "unknown method `x`");
}

#[test]
fn log_macros_format() {
    // Natively without a host the log sink is a no-op; this only has to compile and not panic.
    plugwright_sdk::log::info!("{} + {} = {}", 1, 2, 3);
    plugwright_sdk::warn!("plain");
}

#[test]
fn host_functions_need_a_wasm_host() {
    let err = plugwright_sdk::host::call("upper", b"x").unwrap_err();
    assert_eq!(err, "host function `upper`: host functions are only available to Wasm plugins");
}

/// `#[plugwright(init)]` on a differently named constructor taking the raw config bytes.
struct Raw {
    len: usize,
}

#[plugin(no_export)]
impl Raw {
    #[plugwright(init)]
    fn from_config(config: &[u8]) -> Self {
        Raw { len: config.len() }
    }

    fn len(&self) -> usize {
        self.len
    }

    /// An explicit `self: &Self` receiver and a `mut` binding are plain methods too.
    #[allow(clippy::needless_arbitrary_self_type)]
    fn twice(self: &Self, mut n: u32) -> u32 {
        n *= 2;
        n
    }

    fn bytes(&self, data: Vec<u8>, times: usize) -> Vec<u8> {
        data.repeat(times)
    }

    fn slice(&self) -> &[u8] {
        b"\x00\x01"
    }

    fn maybe(&self, on: bool) -> Option<&'static str> {
        on.then_some("yes")
    }
}

/// The SDK under another name: `crate = path` points the generated code at it.
mod renamed {
    use plugwright_sdk as sdk;

    #[derive(Default)]
    pub struct Renamed;

    #[sdk::plugin(crate = sdk, no_export)]
    impl Renamed {
        fn ping(&self) -> &'static str {
            "pong"
        }
    }
}

/// A fallible constructor whose error is a foreign `Display` type.
struct Strict;

#[plugin(no_export)]
impl Strict {
    fn new(config: &str) -> Result<Self, std::num::ParseIntError> {
        config.trim().parse::<u8>().map(|_| Strict)
    }
}

#[test]
fn constructor_variants() {
    let mut raw = <Raw as Plugin>::new(b"\xff\x00abc").unwrap();
    assert_eq!(call(&mut raw, "len", b"").unwrap(), "5", "raw config bytes are not decoded");
    assert!(<Strict as Plugin>::new(b" 7 ").is_ok());
    assert!(<Strict as Plugin>::new(b"x").err().unwrap().contains("invalid digit"));
    assert!(<Strict as Plugin>::new(b"\xff").err().unwrap().contains("`config` must be UTF-8"));
}

#[test]
fn receivers_bytes_and_options() {
    let mut raw = <Raw as Plugin>::new(b"").unwrap();
    assert_eq!(call(&mut raw, "twice", b"21").unwrap(), "42");
    assert_eq!(raw.call("bytes", b"[[1,2],2]").unwrap(), [1, 2, 1, 2], "Vec<u8> inside JSON args is an array");
    assert_eq!(raw.call("slice", b"").unwrap(), [0, 1]);
    assert_eq!(call(&mut raw, "maybe", b"true").unwrap(), r#""yes""#);
    assert_eq!(call(&mut raw, "maybe", b"false").unwrap(), "null");
    assert!(call(&mut raw, "twice", b"-1").unwrap_err().contains("cannot decode `n`"));
}

#[test]
fn renamed_sdk_path() {
    let mut r = <renamed::Renamed as Plugin>::new(b"{}").unwrap();
    assert_eq!(call(&mut r, "ping", b"").unwrap(), "pong");
}
