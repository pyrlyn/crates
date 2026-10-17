//! `#[interface]`: a typed host-side client for a plugin's methods.

use crate::attrs::{self, Args};
use crate::wire::{self, Wire};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemTrait, Pat, Path, TraitItem, TraitItemFn};

pub fn expand(args: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let args = Args::parse(args, &["crate", "client"])?;
    let mut tr: ItemTrait = syn::parse2(item)?;
    if !tr.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(&tr.generics, "an interface trait cannot be generic"));
    }
    let krate: Path = args.krate.unwrap_or_else(|| syn::parse_quote!(::plugwright));
    let client = args.client.unwrap_or_else(|| format_ident!("{}Client", tr.ident));

    let mut errors: Option<syn::Error> = None;
    let mut push_err = |e: syn::Error| match &mut errors {
        Some(acc) => acc.combine(e),
        None => errors = Some(e),
    };
    let mut names: Vec<String> = Vec::new();
    let mut methods = Vec::new();
    for item in &mut tr.items {
        let f = match item {
            TraitItem::Fn(f) => f,
            other => {
                push_err(syn::Error::new_spanned(other, "an interface trait only declares methods"));
                continue;
            }
        };
        let fa = match attrs::take(&mut f.attrs, &["name"]) {
            Ok(fa) => fa,
            Err(e) => {
                push_err(e);
                continue;
            }
        };
        if f.default.is_some() {
            continue; // a provided method runs on the host; nothing to forward
        }
        let name = fa.name.map(|n| n.value()).unwrap_or_else(|| {
            let s = f.sig.ident.to_string();
            s.strip_prefix("r#").map(str::to_owned).unwrap_or(s)
        });
        if names.contains(&name) {
            push_err(syn::Error::new_spanned(&f.sig.ident, format!("two methods call `{name}`")));
            continue;
        }
        match forward(f, &name, &krate) {
            Ok(m) => {
                names.push(name);
                methods.push(m);
            }
            Err(e) => push_err(e),
        }
    }
    if let Some(e) = errors {
        return Err(e);
    }

    let vis = &tr.vis;
    let ident = &tr.ident;
    let doc = format!("Typed client for the plugin methods of [`{ident}`]; wraps any `Invoke` target.");
    Ok(quote! {
        #tr

        #[doc = #doc]
        #vis struct #client<I> {
            inner: I,
        }

        impl<I> #client<I> {
            pub const fn new(inner: I) -> Self {
                Self { inner }
            }
            pub fn inner(&self) -> &I {
                &self.inner
            }
            pub fn inner_mut(&mut self) -> &mut I {
                &mut self.inner
            }
            pub fn into_inner(self) -> I {
                self.inner
            }
        }

        impl<I: #krate::Invoke> #ident for #client<I> {
            #(#methods)*
        }
    })
}

fn forward(f: &TraitItemFn, name: &str, krate: &Path) -> syn::Result<TokenStream> {
    let pv = quote!(#krate::__private);
    let sig = &f.sig;
    if let Some(a) = &sig.asyncness {
        return Err(syn::Error::new_spanned(a, "interface methods cannot be async"));
    }
    if !sig.generics.params.iter().all(|p| matches!(p, syn::GenericParam::Lifetime(_))) {
        return Err(syn::Error::new_spanned(&sig.generics, "interface methods cannot be generic"));
    }
    match sig.receiver() {
        Some(r) if r.reference.is_some() && r.mutability.is_some() => {}
        _ => return Err(syn::Error::new_spanned(sig, "interface methods take `&mut self`")),
    }
    let rt = wire::ret(&sig.output);
    if !rt.fallible {
        return Err(syn::Error::new_spanned(
            &sig.output,
            "interface methods return `plugwright::Result<T>`: every plugin call can fail",
        ));
    }
    if rt.is_ref {
        return Err(syn::Error::new_spanned(&rt.ok, "interface methods return owned values"));
    }

    // Re-declare the signature with plain argument names.
    let mut sig = sig.clone();
    let mut wires = Vec::new();
    let mut args = Vec::new();
    for (i, arg) in
        sig.inputs.iter_mut().filter_map(|a| if let FnArg::Typed(p) = a { Some(p) } else { None }).enumerate()
    {
        let var = format_ident!("__pw_arg{}", i);
        wires.push(wire::param(&arg.ty)?.wire);
        *arg.pat = Pat::Ident(syn::PatIdent {
            attrs: Vec::new(),
            by_ref: None,
            mutability: None,
            ident: var.clone(),
            subpat: None,
        });
        args.push(var);
    }

    let input = match (wires.as_slice(), args.as_slice()) {
        ([], _) => quote!(let __pw_input: &[u8] = &[];),
        ([Wire::Str], [a]) => quote!(let __pw_input: &[u8] = #a.as_bytes();),
        ([Wire::Bytes], [a]) => quote!(let __pw_input: &[u8] = &#a;),
        ([_], [a]) => quote! {
            let __pw_buf = #pv::encode(#name, &#a)?;
            let __pw_input: &[u8] = &__pw_buf;
        },
        _ => quote! {
            let __pw_buf = #pv::encode_args(#name, &[#(#pv::to_value(#name, &#args)?),*])?;
            let __pw_input: &[u8] = &__pw_buf;
        },
    };
    let output = match rt.wire {
        Wire::Unit => quote! { let _ = __pw_out; ::core::result::Result::Ok(()) },
        Wire::Str => quote!(#pv::utf8(#name, __pw_out).map_err(::core::convert::Into::into)),
        Wire::Bytes => quote!(::core::result::Result::Ok(__pw_out)),
        Wire::Json => quote!(#pv::decode(#name, &__pw_out).map_err(::core::convert::Into::into)),
    };
    Ok(quote! {
        #sig {
            #input
            let __pw_out = #krate::Invoke::invoke(&mut self.inner, #name, __pw_input)?;
            #output
        }
    })
}
