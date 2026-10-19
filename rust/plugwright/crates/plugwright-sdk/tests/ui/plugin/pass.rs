//! Every accepted shape of `#[plugin]` through a renamed SDK path, exported, then called.

use plugwright_sdk as sdk;
use sdk::Plugin;

#[derive(sdk::serde::Deserialize, sdk::serde::Serialize)]
#[serde(crate = "sdk::serde")]
struct Pair {
    a: i64,
    b: i64,
}

struct Calc {
    prefix: String,
}

#[sdk::plugin(crate = sdk)]
impl Calc {
    #[plugwright(init)]
    fn make(prefix: &str) -> Result<Self, std::convert::Infallible> {
        Ok(Calc { prefix: prefix.to_owned() })
    }

    fn add(&self, pair: Pair) -> i64 {
        pair.a + pair.b
    }

    fn swap(&mut self, a: i64, b: &i64) -> Pair {
        Pair { a: *b, b: a }
    }

    fn greet<'a>(&self, name: &'a str) -> String {
        format!("{}{name}", self.prefix)
    }

    fn raw(&self, bytes: Vec<u8>) -> Result<&[u8], String> {
        if bytes.is_empty() { Err("empty".into()) } else { Ok(b"ok") }
    }

    #[plugwright(name = "type")]
    fn kind(&self) {}

    #[plugwright(skip)]
    fn hidden(&self) {}

    fn helper() {}
}

fn main() {
    Calc::helper();
    let mut c = <Calc as Plugin>::new(b"hi ").unwrap();
    c.hidden();
    assert_eq!(c.call("add", br#"{"a":2,"b":3}"#).unwrap(), b"5");
    assert_eq!(c.call("greet", b"you").unwrap(), b"hi you");
    assert_eq!(c.call("type", b"").unwrap(), b"");
    assert!(c.call("missing", b"").is_err());
}
