//! Every accepted shape of `#[interface]` through a renamed facade path, called on a fake target.

use plugwright as pw;
use pw::Invoke;

/// Answers every call with its method name and input, so the test sees what the client sent.
struct Echo;

impl Invoke for Echo {
    fn invoke(&mut self, method: &str, input: &[u8]) -> pw::Result<Vec<u8>> {
        if method == "sum" {
            return Ok(input.to_vec());
        }
        Ok(format!("{method}:{}", String::from_utf8_lossy(input)).into_bytes())
    }
}

#[pw::interface(crate = pw, client = Remote)]
pub trait Calc {
    fn greet(&mut self, name: &str) -> pw::Result<String>;
    fn raw(&mut self, bytes: Vec<u8>) -> pw::Result<Vec<u8>>;
    #[plugwright(name = "sum")]
    fn add<'a>(&mut self, values: &'a [i64]) -> pw::Result<Vec<i64>>;
    fn pair(&mut self, a: i64, b: String) -> pw::Result<String>;
    fn ping(&mut self) -> std::result::Result<String, pw::error::PluginError>;

    fn local(&mut self) -> u8 {
        1
    }
}

fn main() {
    let mut c = Remote::new(Echo);
    assert_eq!(c.greet("you").unwrap(), "greet:you");
    assert_eq!(c.raw(b"x".to_vec()).unwrap(), b"raw:x");
    assert_eq!(c.add(&[1, 2]).unwrap(), [1, 2]);
    assert_eq!(c.pair(1, "b".into()).unwrap(), r#"pair:[1,"b"]"#);
    assert_eq!(c.ping().unwrap(), "ping:");
    assert_eq!(c.local(), 1);
    let _: &mut Echo = c.inner_mut();
    let _: Echo = c.into_inner();
}
