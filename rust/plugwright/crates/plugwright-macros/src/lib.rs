//! Procedural macros for [plugwright](https://crates.io/crates/plugwright).
//!
//! Do not depend on this crate directly: plugin authors use
//! `plugwright_sdk::plugin`, hosts use `plugwright::interface`. Both sides
//! encode arguments and results the same way (see the crate README):
//!
//! * no parameters — the input is ignored;
//! * one parameter — `&str`/`String` travel as UTF-8, `&[u8]`/`Vec<u8>` as raw
//!   bytes, anything else as JSON;
//! * two or more — a JSON array (positional) or object (by parameter name);
//! * results follow the one-parameter rule; `()` is empty output and the `Err`
//!   of a `Result` becomes the plugin error.
//!
//! The classification is syntactic: an alias of `String` travels as JSON.

use proc_macro::TokenStream;

mod attrs;
mod interface;
mod plugin;
mod wire;

/// Export the methods of an inherent `impl` block as a plugwright plugin.
///
/// * Every `&self`/`&mut self` method becomes a plugin method named after the
///   function; `#[plugwright(name = "...")]` renames it, `#[plugwright(skip)]`
///   keeps it private. Associated functions without `self` stay private.
/// * The constructor is `fn new` (or any associated fn marked
///   `#[plugwright(init)]`) taking nothing, `&[u8]`, `&str` or one
///   deserializable config value, and returning `Self` or `Result<Self, E>`.
///   Without one the type must implement `Default`.
/// * `"plugwright.describe"` returns `{"methods":[{"name":..,"params":[..]}]}`.
/// * `#[plugin(no_export)]` implements `Plugin` but leaves `export_plugin!` to
///   you; `#[plugin(crate = path)]` names a renamed SDK.
#[proc_macro_attribute]
pub fn plugin(args: TokenStream, item: TokenStream) -> TokenStream {
    plugin::expand(args.into(), item.into()).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Generate a typed client for a plugin from a trait declaration.
///
/// Each method takes `&mut self` and returns `plugwright::Result<T>`; the trait
/// is kept as written and implemented by `<Trait>Client<I>` for any
/// `I: plugwright::Invoke`. Methods with a default body run on the host and are
/// not forwarded. `#[plugwright(name = "...")]` sets the remote method name,
/// `#[interface(client = Name)]` the client's name and
/// `#[interface(crate = path)]` a renamed facade crate.
#[proc_macro_attribute]
pub fn interface(args: TokenStream, item: TokenStream) -> TokenStream {
    interface::expand(args.into(), item.into()).unwrap_or_else(syn::Error::into_compile_error).into()
}

#[cfg(test)]
mod tests {
    use quote::quote;

    fn plugin_err(item: proc_macro2::TokenStream) -> String {
        crate::plugin::expand(quote!(), item).expect_err("expected a compile error").to_string()
    }

    fn interface_err(item: proc_macro2::TokenStream) -> String {
        crate::interface::expand(quote!(), item).expect_err("expected a compile error").to_string()
    }

    #[test]
    fn plugin_rejects_what_cannot_cross_the_boundary() {
        assert!(plugin_err(quote!(impl Tr for T {})).contains("inherent"));
        assert!(
            plugin_err(quote!(
                impl<X> T<X> {}
            ))
            .contains("generic")
        );
        assert!(plugin_err(quote!(impl T { async fn a(&self) {} })).contains("async"));
        assert!(plugin_err(quote!(impl T { fn a<X>(&self, x: X) {} })).contains("generic"));
        assert!(plugin_err(quote!(impl T { fn a(self) {} })).contains("&self"));
        assert!(plugin_err(quote!(impl T { fn a(&self, x: &mut u8) {} })).contains("&mut"));
        assert!(
            plugin_err(quote!(impl T { fn a(&self) {} #[plugwright(name = "a")] fn b(&self) {} }))
                .contains("two methods")
        );
        assert!(
            plugin_err(quote!(impl T { #[plugwright(name = "plugwright.x")] fn b(&self) {} })).contains("reserved")
        );
        assert!(plugin_err(quote!(impl T { #[plugwright(bogus)] fn b(&self) {} })).contains("unknown argument"));
        assert!(plugin_err(quote!(impl T { fn new(a: u8, b: u8) -> Self { T } })).contains("constructor"));
        assert!(
            plugin_err(quote!(impl T { fn new() -> Self { T } #[plugwright(init)] fn make() -> Self { T } }))
                .contains("one constructor")
        );
        assert!(plugin_err(quote!(impl T { fn a(&self, (x, y): (u8, u8), z: u8) {} })).contains("plain name"));
        let e = crate::plugin::expand(quote!(nope), quote!(impl T {})).err().unwrap().to_string();
        assert!(e.contains("unknown argument"), "{e}");
    }

    #[test]
    fn plugin_expands_and_strips_helper_attributes() {
        let out = crate::plugin::expand(
            quote!(crate = ::my_sdk),
            quote!(impl T { #[plugwright(name = "go")] fn run(&mut self, a: u8, b: &str) -> Result<(), E> { Ok(()) } }),
        )
        .unwrap()
        .to_string();
        assert!(!out.contains("plugwright (name"), "helper attribute left behind: {out}");
        assert!(out.contains(":: my_sdk :: export_plugin !"));
        assert!(out.contains("\"go\" =>"));
        assert!(out.contains(r#"{\"name\":\"go\",\"params\":[\"a\",\"b\"]}"#), "{out}");
    }

    #[test]
    fn interface_requires_fallible_mut_self_methods() {
        assert!(
            interface_err(quote!(
                trait G {
                    fn a(&self) -> Result<u8>;
                }
            ))
            .contains("&mut self")
        );
        assert!(
            interface_err(quote!(
                trait G {
                    fn a(&mut self) -> u8;
                }
            ))
            .contains("Result")
        );
        assert!(
            interface_err(quote!(
                trait G {
                    fn a(&mut self) -> Result<&str>;
                }
            ))
            .contains("owned")
        );
        assert!(
            interface_err(quote!(
                trait G {
                    type X;
                }
            ))
            .contains("only declares methods")
        );
        assert!(
            interface_err(quote!(
                trait G<T> {}
            ))
            .contains("generic")
        );
        let ok = crate::interface::expand(
            quote!(client = Remote),
            quote!(
                pub trait G {
                    fn a(&mut self, x: &str) -> Result<String>;
                    fn local(&mut self) -> u8 {
                        1
                    }
                }
            ),
        )
        .unwrap()
        .to_string();
        assert!(ok.contains("pub struct Remote"));
        assert!(!ok.contains("fn local (& mut self) -> u8 { let"), "provided methods are not forwarded");
    }
}
