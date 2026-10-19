//! The wire convention shared by `#[plugin]` and `#[interface]`.
//!
//! Classification is syntactic: the macros see tokens, not resolved types, so a
//! type alias for `String` is encoded as JSON, not as UTF-8.
//!
//! * no parameters — the input is ignored;
//! * one parameter — `&str`/`String` travel as UTF-8, `&[u8]`/`Vec<u8>` as raw
//!   bytes, anything else as JSON;
//! * two or more — a JSON array (positional) or object (by parameter name);
//! * return values follow the one-parameter rule; `()` is empty output, and the
//!   error of a `Result<T, E>` becomes the plugin error (`E: Display`).

use syn::{GenericArgument, PathArguments, ReturnType, Type, TypePath};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Unit,
    Str,
    Bytes,
    Json,
}

/// A parameter type, split into how it travels and how it is held.
pub struct ParamTy {
    pub wire: Wire,
    /// The type a decoded value is stored in (`str` → `String`, `&T` → `T`).
    pub owned: Type,
    /// The declared type is a shared reference; pass `&value`.
    pub by_ref: bool,
    /// `&str` / `&[u8]`: a single argument can borrow the input without copying.
    pub borrows_input: bool,
}

/// The shape of a return type.
pub struct RetTy {
    pub wire: Wire,
    /// The success type, after unwrapping `Result`.
    pub ok: Type,
    pub is_ref: bool,
    pub fallible: bool,
}

fn strip(ty: &Type) -> &Type {
    match ty {
        Type::Paren(p) => strip(&p.elem),
        Type::Group(g) => strip(&g.elem),
        t => t,
    }
}

fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match strip(ty) {
        Type::Path(TypePath { qself: None, path }) => path.segments.last(),
        _ => None,
    }
}

fn is_ident(ty: &Type, name: &str) -> bool {
    last_segment(ty).is_some_and(|s| s.ident == name && s.arguments.is_none())
}

fn single_type_arg(seg: &syn::PathSegment) -> Option<&Type> {
    match &seg.arguments {
        PathArguments::AngleBracketed(a) => a.args.iter().find_map(|g| match g {
            GenericArgument::Type(t) => Some(t),
            _ => None,
        }),
        _ => None,
    }
}

fn is_vec_u8(ty: &Type) -> bool {
    last_segment(ty).is_some_and(|s| s.ident == "Vec" && single_type_arg(s).is_some_and(|t| is_ident(t, "u8")))
}

fn is_u8_slice(ty: &Type) -> bool {
    matches!(strip(ty), Type::Slice(s) if is_ident(&s.elem, "u8"))
}

fn is_unit(ty: &Type) -> bool {
    matches!(strip(ty), Type::Tuple(t) if t.elems.is_empty())
}

/// How a value of this (non-reference) type travels.
fn value_wire(ty: &Type) -> Wire {
    if is_unit(ty) {
        Wire::Unit
    } else if is_ident(ty, "String") || is_ident(ty, "str") {
        Wire::Str
    } else if is_vec_u8(ty) || is_u8_slice(ty) {
        Wire::Bytes
    } else {
        Wire::Json
    }
}

pub fn param(ty: &Type) -> syn::Result<ParamTy> {
    match strip(ty) {
        Type::Reference(r) => {
            if r.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    ty,
                    "`&mut` parameters cannot cross the plugin boundary; take the value or `&T`",
                ));
            }
            let inner = strip(&r.elem);
            let (owned, borrows_input): (Type, bool) = if is_ident(inner, "str") {
                (syn::parse_quote!(::std::string::String), true)
            } else if is_u8_slice(inner) {
                (syn::parse_quote!(::std::vec::Vec<u8>), true)
            } else {
                (inner.clone(), false)
            };
            Ok(ParamTy { wire: value_wire(inner), owned, by_ref: true, borrows_input })
        }
        Type::ImplTrait(_) | Type::TraitObject(_) | Type::Infer(_) => {
            Err(syn::Error::new_spanned(ty, "parameters must have a concrete type that serde can decode"))
        }
        t => Ok(ParamTy { wire: value_wire(t), owned: t.clone(), by_ref: false, borrows_input: false }),
    }
}

/// `Result<T>` / `Result<T, E>` (any path ending in `Result`) → `T`.
pub fn result_ok(ty: &Type) -> Option<&Type> {
    last_segment(ty).filter(|s| s.ident == "Result").and_then(single_type_arg)
}

pub fn ret(output: &ReturnType) -> RetTy {
    let ty: Type = match output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, t) => (**t).clone(),
    };
    let (ok, fallible) = match result_ok(&ty) {
        Some(ok) => (ok.clone(), true),
        None => (ty, false),
    };
    let (wire, is_ref) = match strip(&ok) {
        Type::Reference(r) => (value_wire(&r.elem), true),
        t => (value_wire(t), false),
    };
    RetTy { wire, ok, is_ref, fallible }
}

/// Minimal JSON string escaping for the compile-time `describe` document.
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn ty(s: &str) -> Type {
        syn::parse_str(s).unwrap()
    }

    fn ret_of(s: &str) -> RetTy {
        let f: syn::ItemFn = syn::parse_str(&format!("fn f() {s} {{}}")).unwrap();
        ret(&f.sig.output)
    }

    fn tokens(t: &Type) -> String {
        t.to_token_stream().to_string()
    }

    #[test]
    fn single_parameters_are_classified_by_their_spelling() {
        let cases = [
            ("&str", Wire::Str, ":: std :: string :: String", true, true),
            ("&'a str", Wire::Str, ":: std :: string :: String", true, true),
            ("String", Wire::Str, "String", false, false),
            ("std::string::String", Wire::Str, "std :: string :: String", false, false),
            ("&String", Wire::Str, "String", true, false),
            ("&[u8]", Wire::Bytes, ":: std :: vec :: Vec < u8 >", true, true),
            ("Vec<u8>", Wire::Bytes, "Vec < u8 >", false, false),
            ("&Vec<u8>", Wire::Bytes, "Vec < u8 >", true, false),
            ("(Vec<u8>)", Wire::Bytes, "Vec < u8 >", false, false),
            ("Vec<u16>", Wire::Json, "Vec < u16 >", false, false),
            ("u64", Wire::Json, "u64", false, false),
            ("&Config", Wire::Json, "Config", true, false),
            ("Option<String>", Wire::Json, "Option < String >", false, false),
            ("Box<str>", Wire::Json, "Box < str >", false, false),
            ("MyString", Wire::Json, "MyString", false, false),
            ("()", Wire::Unit, "()", false, false),
        ];
        for (src, wire, owned, by_ref, borrows) in cases {
            let p = param(&ty(src)).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert!(p.wire == wire, "{src}: wrong wire");
            assert_eq!(tokens(&p.owned), owned, "{src}: owned type");
            assert_eq!((p.by_ref, p.borrows_input), (by_ref, borrows), "{src}: by_ref/borrows");
        }
    }

    #[test]
    fn parameters_that_cannot_be_decoded_are_rejected() {
        for src in ["&mut str", "&mut Vec<u8>", "impl Into<String>", "dyn Fn()", "_"] {
            assert!(param(&ty(src)).is_err(), "{src} must be rejected");
        }
    }

    #[test]
    fn return_types_unwrap_result_and_references() {
        let r = ret_of("");
        assert!(r.wire == Wire::Unit && !r.fallible && !r.is_ref);
        let r = ret_of("-> String");
        assert!(r.wire == Wire::Str && !r.fallible);
        let r = ret_of("-> &'static str");
        assert!(r.wire == Wire::Str && r.is_ref);
        let r = ret_of("-> &[u8]");
        assert!(r.wire == Wire::Bytes && r.is_ref);
        let r = ret_of("-> Result<Vec<u8>, String>");
        assert!(r.wire == Wire::Bytes && r.fallible);
        assert_eq!(tokens(&r.ok), "Vec < u8 >");
        let r = ret_of("-> std::io::Result<()>");
        assert!(r.wire == Wire::Unit && r.fallible, "single-argument Result aliases count");
        let r = ret_of("-> plugwright::Result<Sum>");
        assert!(r.wire == Wire::Json && r.fallible);
        let r = ret_of("-> Option<String>");
        assert!(r.wire == Wire::Json && !r.fallible, "only `Result` is unwrapped");
        let r = ret_of("-> Result<&str, E>");
        assert!(r.wire == Wire::Str && r.is_ref && r.fallible);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("plain"), r#""plain""#);
        assert_eq!(json_str(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(json_str("tab\tnl\n"), r#""tab\u0009nl\u000a""#);
        assert_eq!(json_str("ünï"), "\"ünï\"");
    }
}
