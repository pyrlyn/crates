//! Typed calls: the [`Invoke`] trait behind `#[plugwright::interface]` clients.

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{PluginError, Result};
use crate::manager::PluginManager;
use crate::plugin::Plugin;

/// Reserved method answered by every `#[plugin]`-generated plugin.
pub const DESCRIBE: &str = "plugwright.describe";

/// Something that can perform a raw plugin call.
pub trait Invoke {
    fn invoke(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>>;
}

impl Invoke for dyn Plugin + '_ {
    fn invoke(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        self.call(method, input)
    }
}

impl<T: Invoke + ?Sized> Invoke for &mut T {
    fn invoke(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        (**self).invoke(method, input)
    }
}

impl<T: Invoke + ?Sized> Invoke for Box<T> {
    fn invoke(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        (**self).invoke(method, input)
    }
}

/// A plugin id bound to its manager. Calls go through [`PluginManager::call`],
/// so they get the same logging, timeouts and restarts.
pub struct Bound<'m> {
    pub(crate) manager: &'m mut PluginManager,
    pub(crate) id: String,
}

impl Bound<'_> {
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Invoke for Bound<'_> {
    fn invoke(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        self.manager.call(&self.id, method, input)
    }
}

/// One entry of a plugin's `plugwright.describe` answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MethodInfo {
    pub name: String,
    #[serde(default)]
    pub params: Vec<String>,
}

/// Ask a plugin which methods it exports. Plugins written without `#[plugin]`
/// usually answer with a [`PluginError::Call`] for an unknown method.
pub fn describe(target: &mut (impl Invoke + ?Sized)) -> Result<Vec<MethodInfo>> {
    #[derive(Deserialize)]
    struct Doc {
        methods: Vec<MethodInfo>,
    }
    let out = target.invoke(DESCRIBE, b"")?;
    let doc: Doc = serde_json::from_slice(&out)
        .map_err(|e| PluginError::Protocol(format!("`{DESCRIBE}` returned malformed output: {e}")))?;
    Ok(doc.methods)
}

/// Runtime support for `#[interface]`; not a public API.
#[doc(hidden)]
pub mod __private {
    use super::*;
    pub use serde_json;

    fn encode_err(method: &str, e: serde_json::Error) -> PluginError {
        PluginError::Protocol(format!("cannot encode arguments of `{method}`: {e}"))
    }

    pub fn encode<T: Serialize + ?Sized>(method: &str, value: &T) -> Result<Vec<u8>> {
        serde_json::to_vec(value).map_err(|e| encode_err(method, e))
    }

    pub fn to_value<T: Serialize + ?Sized>(method: &str, value: &T) -> Result<serde_json::Value> {
        serde_json::to_value(value).map_err(|e| encode_err(method, e))
    }

    pub fn encode_args(method: &str, values: &[serde_json::Value]) -> Result<Vec<u8>> {
        encode(method, values)
    }

    pub fn utf8(method: &str, out: Vec<u8>) -> Result<String> {
        String::from_utf8(out).map_err(|e| PluginError::Protocol(format!("`{method}` returned non-UTF-8 text: {e}")))
    }

    pub fn decode<T: DeserializeOwned>(method: &str, out: &[u8]) -> Result<T> {
        serde_json::from_slice(out)
            .map_err(|e| PluginError::Protocol(format!("`{method}` returned unexpected output: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::plugin::{Backend, PluginInfo};

    /// Answers every method from a fixed table and records what was called.
    struct Fake {
        info: PluginInfo,
        answers: BTreeMap<&'static str, Vec<u8>>,
        calls: Vec<(String, Vec<u8>)>,
    }

    fn fake(answers: &[(&'static str, &[u8])]) -> Fake {
        Fake {
            info: PluginInfo {
                name: "fake".into(),
                version: semver::Version::new(1, 0, 0),
                sdk_version: semver::Version::new(0, 1, 0),
                backend: Backend::Native,
                engine: None,
                sandbox: None,
            },
            answers: answers.iter().map(|(m, out)| (*m, out.to_vec())).collect(),
            calls: Vec::new(),
        }
    }

    impl Plugin for Fake {
        fn info(&self) -> &PluginInfo {
            &self.info
        }

        fn call(&mut self, method: &str, input: &[u8]) -> Result<Vec<u8>> {
            self.calls.push((method.to_owned(), input.to_vec()));
            self.answers
                .get(method)
                .cloned()
                .ok_or_else(|| PluginError::Call { method: method.to_owned(), message: "unknown method".into() })
        }
    }

    fn via(target: &mut impl Invoke, method: &str, input: &[u8]) -> Result<Vec<u8>> {
        target.invoke(method, input)
    }

    #[test]
    fn invoke_forwards_through_dyn_plugin_box_and_mut_ref() {
        let mut boxed: Box<dyn Plugin> = Box::new(fake(&[("echo", b"pong")]));
        assert_eq!(via(&mut boxed, "echo", b"1").unwrap(), b"pong");
        let by_ref: &mut dyn Plugin = boxed.as_mut();
        assert_eq!(via(&mut &mut *by_ref, "echo", b"2").unwrap(), b"pong");
        assert_eq!(by_ref.invoke("echo", b"3").unwrap(), b"pong");
        let mut nested: Box<&mut dyn Plugin> = Box::new(by_ref);
        assert_eq!(via(&mut nested, "echo", b"4").unwrap(), b"pong");
        assert!(matches!(
            via(&mut boxed, "nope", b""),
            Err(PluginError::Call { method, .. }) if method == "nope"
        ));
    }

    #[test]
    fn invoke_passes_method_and_input_unchanged() {
        let mut f = fake(&[("m", b"")]);
        {
            let p: &mut dyn Plugin = &mut f;
            via(&mut Box::new(p), "m", b"\x00\xffbytes").unwrap();
        }
        assert_eq!(f.calls, [("m".to_owned(), b"\x00\xffbytes".to_vec())]);
    }

    #[test]
    fn describe_parses_methods_and_defaults_params() {
        let doc = br#"{"methods":[{"name":"greet","params":["name"]},{"name":"ping","extra":1}],"v":2}"#;
        let mut p: Box<dyn Plugin> = Box::new(fake(&[(DESCRIBE, doc)]));
        let methods = describe(&mut p).unwrap();
        assert_eq!(
            methods,
            [
                MethodInfo { name: "greet".into(), params: vec!["name".into()] },
                MethodInfo { name: "ping".into(), params: vec![] }
            ]
        );
        let mut empty: Box<dyn Plugin> = Box::new(fake(&[(DESCRIBE, br#"{"methods":[]}"#)]));
        assert!(describe(&mut empty).unwrap().is_empty());
    }

    #[test]
    fn describe_reports_malformed_output_as_protocol_error() {
        for out in [&b""[..], b"not json", b"{}", br#"{"methods":{}}"#, br#"{"methods":[{"params":[]}]}"#, b"\xff"] {
            let mut p: Box<dyn Plugin> = Box::new(fake(&[(DESCRIBE, out)]));
            match describe(&mut p) {
                Err(PluginError::Protocol(m)) => assert!(m.contains("`plugwright.describe` returned malformed"), "{m}"),
                other => panic!("{out:?}: expected Protocol, got {other:?}"),
            }
        }
    }

    #[test]
    fn describe_passes_plugin_errors_through() {
        let mut p: Box<dyn Plugin> = Box::new(fake(&[]));
        assert!(matches!(describe(&mut p), Err(PluginError::Call { method, .. }) if method == DESCRIBE));
    }

    #[test]
    fn private_encoders_produce_json_and_map_failures_to_protocol() {
        use super::__private::*;
        assert_eq!(encode("m", &("a", 1)).unwrap(), br#"["a",1]"#);
        assert_eq!(encode("m", "s").unwrap(), br#""s""#);
        let args = [to_value("m", &1).unwrap(), to_value("m", &"x").unwrap()];
        assert_eq!(encode_args("m", &args).unwrap(), br#"[1,"x"]"#);
        assert_eq!(encode_args("m", &[]).unwrap(), b"[]");

        // JSON object keys must be strings; a tuple key cannot be encoded.
        let bad: BTreeMap<(u8, u8), u8> = [((1, 2), 3)].into();
        for err in [encode("add", &bad).unwrap_err(), to_value("add", &bad).unwrap_err()] {
            match err {
                PluginError::Protocol(m) => assert!(m.starts_with("cannot encode arguments of `add`: "), "{m}"),
                other => panic!("expected Protocol, got {other:?}"),
            }
        }
    }

    #[test]
    fn private_decoders_map_failures_to_protocol() {
        use super::__private::*;
        assert_eq!(utf8("greet", b"hi".to_vec()).unwrap(), "hi");
        assert!(matches!(
            utf8("greet", vec![0xff, 0xfe]),
            Err(PluginError::Protocol(m)) if m.starts_with("`greet` returned non-UTF-8 text")
        ));
        assert_eq!(decode::<Vec<u32>>("sum", b"[1,2]").unwrap(), [1, 2]);
        assert_eq!(decode::<Option<u8>>("sum", b"null").unwrap(), None);
        for out in [&b""[..], b"[1,", b"\"text\"", b"[1,-2]"] {
            assert!(matches!(
                decode::<Vec<u32>>("sum", out),
                Err(PluginError::Protocol(m)) if m.starts_with("`sum` returned unexpected output")
            ));
        }
    }
}
