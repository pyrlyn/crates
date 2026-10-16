//! `#[injectable]` on an inherent `impl` block: the one `#[inject]`
//! constructor's parameters become dependencies. Used when a type keeps
//! invariants in its constructor or has private fields.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{FnArg, ImplItem, ItemImpl, ReturnType, Type, spanned::Spanned};

use crate::deps::{Source, check_dependency, injectable_impl};
use crate::derive::combine;

pub(crate) fn expand(attr: &TokenStream, item: &ItemImpl) -> TokenStream {
    match try_expand(attr, item) {
        Ok(tokens) => tokens,
        Err(err) => {
            // Emit the impl without its `#[inject]` markers so the user sees
            // only this error, not a follow-up "cannot find attribute".
            let item = strip_inject(item).to_token_stream();
            let err = err.to_compile_error();
            quote!(#item #err)
        }
    }
}

/// The impl block with every `#[inject]` marker removed.
fn strip_inject(item: &ItemImpl) -> ItemImpl {
    let mut item = item.clone();
    for impl_item in &mut item.items {
        if let ImplItem::Fn(func) = impl_item {
            func.attrs.retain(|a| !a.path().is_ident("inject"));
        }
    }
    item
}

fn try_expand(attr: &TokenStream, item: &ItemImpl) -> syn::Result<TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new(
            attr.span(),
            "`#[injectable]` takes no arguments",
        ));
    }
    if let Some((_, path, _)) = &item.trait_ {
        return Err(syn::Error::new(
            path.span(),
            "put `#[injectable]` on an inherent `impl Type { .. }` block, not on a trait impl",
        ));
    }
    let marked: Vec<usize> = item
        .items
        .iter()
        .enumerate()
        .filter(|(_, impl_item)| {
            matches!(impl_item, ImplItem::Fn(func) if func.attrs.iter().any(|a| a.path().is_ident("inject")))
        })
        .map(|(index, _)| index)
        .collect();
    let item = strip_inject(item);
    let index = match marked.as_slice() {
        [index] => *index,
        [] => {
            return Err(syn::Error::new(
                item.self_ty.span(),
                "mark the constructor with `#[inject]`: `#[inject] fn new(dep: Dep) -> Self`",
            ));
        }
        [_, second, ..] => {
            return Err(syn::Error::new(
                item.items[*second].span(),
                "only one constructor can be `#[inject]`; register other ways of building the type as factories in `container!`",
            ));
        }
    };
    let ImplItem::Fn(func) = &item.items[index] else {
        return Err(syn::Error::new(
            item.span(),
            "internal: marked item is not a function",
        ));
    };
    let sig = &func.sig;
    check_signature(sig, &item.self_ty)?;

    let mut errors = None;
    let mut sources = Vec::with_capacity(sig.inputs.len());
    for input in &sig.inputs {
        if let FnArg::Typed(arg) = input {
            match check_dependency(&arg.ty) {
                Ok(()) => sources.push(Source::Resolve((*arg.ty).clone())),
                Err(err) => combine(&mut errors, err),
            }
        }
    }
    if let Some(err) = errors {
        return Err(err);
    }

    let name = &sig.ident;
    let self_ty = item.self_ty.to_token_stream();
    let injectable = injectable_impl(
        &self_ty,
        &item.generics,
        &sources,
        |values| quote!(Self::#name(#(#values),*)),
    );
    Ok(quote!(#item #injectable))
}

fn check_signature(sig: &syn::Signature, self_ty: &Type) -> syn::Result<()> {
    if let Some(receiver) = sig.receiver() {
        return Err(syn::Error::new(
            receiver.span(),
            "an `#[inject]` constructor must not take `self`",
        ));
    }
    if let Some(token) = &sig.asyncness {
        return Err(syn::Error::new(
            token.span(),
            "async constructors are not supported: build the value before the container and register it as `instance`",
        ));
    }
    if !sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            sig.generics.span(),
            "an `#[inject]` constructor cannot have its own generic parameters; put them on the `impl` block",
        ));
    }
    let returns_self = match &sig.output {
        ReturnType::Type(_, ty) => match &**ty {
            Type::Path(path) => {
                path.path.is_ident("Self")
                    || ty.to_token_stream().to_string() == self_ty.to_token_stream().to_string()
            }
            _ => false,
        },
        ReturnType::Default => false,
    };
    if returns_self {
        Ok(())
    } else {
        Err(syn::Error::new(
            sig.output.span(),
            "an `#[inject]` constructor must return `Self`; fallible or async setup belongs before the container: build the value and register it as `instance`",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::try_expand;
    use proc_macro2::TokenStream;

    fn error_of(src: &str) -> String {
        let item = syn::parse_str(src).unwrap();
        try_expand(&TokenStream::new(), &item)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn missing_marker_is_rejected_with_example() {
        assert!(error_of("impl S { fn new() -> Self { S } }").contains("#[inject] fn new"));
    }

    #[test]
    fn two_markers_are_rejected() {
        let src = "impl S { #[inject] fn a() -> Self { S } #[inject] fn b() -> Self { S } }";
        assert!(error_of(src).contains("only one constructor"));
    }

    #[test]
    fn fallible_constructor_is_rejected_with_instance_hint() {
        let src = "impl S { #[inject] fn new() -> Result<Self, E> { todo!() } }";
        assert!(error_of(src).contains("register it as `instance`"));
    }

    #[test]
    fn async_constructor_is_rejected() {
        assert!(
            error_of("impl S { #[inject] async fn new() -> Self { S } }")
                .contains("async constructors")
        );
    }

    #[test]
    fn marker_is_removed_from_emitted_impl() {
        let item = syn::parse_str("impl S { #[inject] fn new(a: A) -> Self { S } }").unwrap();
        let out = try_expand(&TokenStream::new(), &item).unwrap().to_string();
        assert!(!out.contains("# [inject]"));
        assert!(out.contains("Self :: new"));
    }
}
