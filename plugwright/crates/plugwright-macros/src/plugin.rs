//! `#[plugin]`: turn an inherent `impl` block into a plugwright plugin.

use crate::attrs::{self, Args};
use crate::wire::{self, Wire};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, Pat, Path};

struct Method {
    name: String,
    params: Vec<String>,
    arm: TokenStream,
}

pub fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = Args::parse(args, &["crate", "no_export"])?;
    let mut imp: ItemImpl = syn::parse2(item)?;
    if let Some((_, path, _)) = &imp.trait_ {
        return Err(syn::Error::new_spanned(path, "#[plugin] goes on an inherent `impl Type { ... }` block"));
    }
    if !imp.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&imp.generics, "a plugin type cannot be generic"));
    }
    let krate: Path = args.krate.unwrap_or_else(|| syn::parse_quote!(::plugwright_sdk));
    let pv = quote!(#krate::__private);

    let mut errors: Option<syn::Error> = None;
    let mut push_err = |e: syn::Error| match &mut errors {
        Some(acc) => acc.combine(e),
        None => errors = Some(e),
    };
    let mut ctor: Option<TokenStream> = None;
    let mut methods: Vec<Method> = Vec::new();

    for item in &mut imp.items {
        let ImplItem::Fn(f) = item else { continue };
        let fa = match attrs::take(&mut f.attrs, &["skip", "init", "name"]) {
            Ok(fa) => fa,
            Err(e) => {
                push_err(e);
                continue;
            }
        };
        if fa.skip {
            continue;
        }
        let has_receiver = f.sig.receiver().is_some();
        if fa.init || (!has_receiver && f.sig.ident == "new") {
            if has_receiver {
                push_err(syn::Error::new_spanned(&f.sig, "the constructor cannot take `self`"));
            } else if ctor.is_some() {
                push_err(syn::Error::new_spanned(&f.sig, "a plugin has one constructor"));
            } else {
                match constructor(f, &pv) {
                    Ok(t) => ctor = Some(t),
                    Err(e) => push_err(e),
                }
            }
            continue;
        }
        if !has_receiver {
            continue; // associated helper, not part of the interface
        }
        let name = fa.name.map(|n| n.value()).unwrap_or_else(|| f.sig.ident.unraw_string());
        if name.starts_with("plugwright.") {
            push_err(syn::Error::new_spanned(&f.sig.ident, "method names starting with `plugwright.` are reserved"));
            continue;
        }
        if methods.iter().any(|m| m.name == name) {
            push_err(syn::Error::new_spanned(&f.sig.ident, format!("two methods are exported as `{name}`")));
            continue;
        }
        match method(f, name, &pv) {
            Ok(m) => methods.push(m),
            Err(e) => push_err(e),
        }
    }
    if let Some(e) = errors {
        return Err(e);
    }

    let ty = &imp.self_ty;
    let ctor = ctor.unwrap_or_else(|| {
        quote_spanned! {ty.span()=>
            let _ = config;
            ::core::result::Result::Ok(<Self as ::core::default::Default>::default())
        }
    });
    let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
    let arms = methods.iter().map(|m| &m.arm);
    let describe = describe(&methods);
    let export = (!args.no_export).then(|| quote!(#krate::export_plugin!(#ty);));

    Ok(quote! {
        #imp

        impl #krate::Plugin for #ty {
            fn new(config: &[u8]) -> ::core::result::Result<Self, ::std::string::String> {
                #ctor
            }

            fn call(
                &mut self,
                method: &str,
                input: &[u8],
            ) -> ::core::result::Result<::std::vec::Vec<u8>, ::std::string::String> {
                match method {
                    #(#arms)*
                    "plugwright.describe" => ::core::result::Result::Ok(#describe.as_bytes().to_vec()),
                    other => ::core::result::Result::Err(#pv::unknown_method(other, &[#(#names),*])),
                }
            }
        }

        #export
    })
}

trait UnrawString {
    fn unraw_string(&self) -> String;
}

impl UnrawString for syn::Ident {
    fn unraw_string(&self) -> String {
        let s = self.to_string();
        s.strip_prefix("r#").map(str::to_owned).unwrap_or(s)
    }
}

fn check_sig(f: &ImplItemFn) -> syn::Result<()> {
    if let Some(a) = &f.sig.asyncness {
        return Err(syn::Error::new_spanned(a, "plugin methods cannot be async"));
    }
    if !f.sig.generics.params.iter().all(|p| matches!(p, syn::GenericParam::Lifetime(_))) {
        return Err(syn::Error::new_spanned(&f.sig.generics, "plugin methods cannot be generic"));
    }
    if let Some(v) = &f.sig.variadic {
        return Err(syn::Error::new_spanned(v, "plugin methods cannot be variadic"));
    }
    Ok(())
}

fn constructor(f: &ImplItemFn, pv: &TokenStream) -> syn::Result<TokenStream> {
    check_sig(f)?;
    let ident = &f.sig.ident;
    let inputs: Vec<&syn::PatType> = typed_inputs(f)?;
    let call = match inputs.as_slice() {
        [] => quote! { { let _ = config; Self::#ident() } },
        [p] => {
            let pt = wire::param(&p.ty)?;
            let value = match (pt.wire, pt.borrows_input) {
                (Wire::Bytes, true) => quote!(config),
                (Wire::Str, true) => quote!(#pv::utf8(config, "config")?),
                _ => {
                    let owned = &pt.owned;
                    let amp = pt.by_ref.then(|| quote!(&));
                    quote!(#amp #pv::json::<#owned>(config, "config")?)
                }
            };
            quote!(Self::#ident(#value))
        }
        _ => {
            return Err(syn::Error::new_spanned(
                &f.sig.inputs,
                "the constructor takes nothing, `&[u8]`, `&str` or one deserializable config value",
            ));
        }
    };
    let rt = wire::ret(&f.sig.output);
    Ok(if rt.fallible {
        quote!(#call.map_err(|e| ::std::string::ToString::to_string(&e)))
    } else {
        quote!(::core::result::Result::Ok(#call))
    })
}

fn typed_inputs(f: &ImplItemFn) -> syn::Result<Vec<&syn::PatType>> {
    f.sig
        .inputs
        .iter()
        .filter_map(|a| match a {
            FnArg::Typed(p) => Some(Ok(p)),
            FnArg::Receiver(r) if !matches!(*r.ty, syn::Type::Reference(_)) => {
                Some(Err(syn::Error::new_spanned(r, "plugin methods take `&self` or `&mut self`")))
            }
            FnArg::Receiver(_) => None,
        })
        .collect()
}

fn method(f: &ImplItemFn, name: String, pv: &TokenStream) -> syn::Result<Method> {
    check_sig(f)?;
    let ident = &f.sig.ident;
    let inputs = typed_inputs(f)?;
    let mut params = Vec::with_capacity(inputs.len());
    let mut decode = Vec::with_capacity(inputs.len());
    let mut pass = Vec::with_capacity(inputs.len());

    for (i, p) in inputs.iter().enumerate() {
        let pname = match &*p.pat {
            Pat::Ident(pi) => pi.ident.unraw_string(),
            _ if inputs.len() == 1 => "input".to_owned(),
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "use a plain name: it is the key in the JSON argument object",
                ));
            }
        };
        let pt = wire::param(&p.ty)?;
        let var = format_ident!("__pw_arg{}", i);
        let owned = &pt.owned;
        let amp = pt.by_ref.then(|| quote!(&));
        if inputs.len() == 1 {
            match (pt.wire, pt.borrows_input) {
                (Wire::Bytes, true) => decode.push(quote!(let #var = input;)),
                (Wire::Str, true) => decode.push(quote!(let #var = #pv::utf8(input, #pname)?;)),
                (Wire::Bytes, false) => {
                    decode.push(quote!(let #var: #owned = input.to_vec();));
                }
                (Wire::Str, false) => {
                    decode.push(quote!(let #var: #owned = #pv::utf8(input, #pname)?.to_owned();));
                }
                _ => decode.push(quote!(let #var: #owned = #pv::json(input, #pname)?;)),
            }
            pass.push(if pt.borrows_input { quote!(#var) } else { quote!(#amp #var) });
        } else {
            decode.push(quote!(let #var: #owned = __pw_args.take(#i, #pname)?;));
            pass.push(quote!(#amp #var));
        }
        params.push(pname);
    }
    if inputs.len() > 1 {
        let n = inputs.len();
        decode.insert(0, quote!(let mut __pw_args = #pv::Args::parse(input, #n)?;));
        decode.push(quote!(__pw_args.finish()?;));
    }
    if inputs.is_empty() {
        decode.push(quote!(let _ = input;));
    }

    let rt = wire::ret(&f.sig.output);
    let value = if rt.fallible {
        quote!(Self::#ident(self, #(#pass),*).map_err(|e| ::std::string::ToString::to_string(&e))?)
    } else {
        quote!(Self::#ident(self, #(#pass),*))
    };
    let encode = encode(rt.wire, rt.is_ref, pv);
    let arm = quote! {
        #name => {
            #(#decode)*
            let __pw_out = #value;
            #encode
        }
    };
    Ok(Method { name, params, arm })
}

/// Turn `__pw_out` into the call's output bytes.
fn encode(wire: Wire, is_ref: bool, pv: &TokenStream) -> TokenStream {
    match (wire, is_ref) {
        (Wire::Unit, _) => quote! { let () = __pw_out; ::core::result::Result::Ok(::std::vec::Vec::new()) },
        (Wire::Str, true) => quote!(::core::result::Result::Ok(<str>::as_bytes(__pw_out).to_vec())),
        (Wire::Str, false) => {
            quote!(::core::result::Result::Ok(::std::string::String::from(__pw_out).into_bytes()))
        }
        (Wire::Bytes, true) => quote!(::core::result::Result::Ok(<[u8]>::to_vec(__pw_out))),
        (Wire::Bytes, false) => quote!(::core::result::Result::Ok(::std::vec::Vec::<u8>::from(__pw_out))),
        (Wire::Json, _) => quote!(#pv::to_json(&__pw_out)),
    }
}

/// `{"methods":[{"name":..,"params":[..]}]}`, built at compile time.
fn describe(methods: &[Method]) -> String {
    let items: Vec<String> = methods
        .iter()
        .map(|m| {
            let params: Vec<String> = m.params.iter().map(|p| wire::json_str(p)).collect();
            format!("{{\"name\":{},\"params\":[{}]}}", wire::json_str(&m.name), params.join(","))
        })
        .collect();
    format!("{{\"methods\":[{}]}}", items.join(","))
}
