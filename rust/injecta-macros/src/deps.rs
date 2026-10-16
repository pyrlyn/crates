//! Code shared by `#[derive(Injectable)]` and `#[injectable]`: given the self
//! type and its dependency types, emit the `Injectable<C>` impl with one
//! `C: Provide<Dep>` bound per dependency and the compile-time `DEPTH`.

use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use syn::{Generics, Ident, Type, spanned::Spanned};

/// One way a value is obtained inside `inject`.
pub(crate) enum Source {
    /// Resolved from the container.
    Resolve(Type),
    /// An expression written by the user (`#[inject(value = ..)]`/`default`).
    Expr(TokenStream),
}

/// Emits `impl Injectable<C> for SelfTy`, where `build` turns the per-source
/// expressions into the constructor call.
pub(crate) fn injectable_impl(
    self_ty: &TokenStream,
    generics: &Generics,
    sources: &[Source],
    build: impl FnOnce(&[TokenStream]) -> TokenStream,
) -> TokenStream {
    let c = Ident::new("__InjectaC", Span::call_site());
    let mut impl_generics = generics.clone();
    impl_generics
        .params
        .push(syn::parse_quote!(#c: ?::core::marker::Sized));
    let (impl_g, _, _) = impl_generics.split_for_impl();
    let mut where_clause = generics
        .where_clause
        .clone()
        .unwrap_or_else(|| syn::parse_quote!(where));

    let mut depth = quote!(0usize);
    let mut values = Vec::with_capacity(sources.len());
    for source in sources {
        match source {
            Source::Resolve(ty) => {
                where_clause
                    .predicates
                    .push(syn::parse_quote_spanned!(ty.span()=> #c: ::injecta::Provide<#ty>));
                depth = quote!(::injecta::__private::depth_max(<#c as ::injecta::Provide<#ty>>::DEPTH, #depth));
                values.push(
                    quote_spanned!(ty.span()=> <#c as ::injecta::Provide<#ty>>::provide(container)),
                );
            }
            Source::Expr(expr) => values.push(expr.clone()),
        }
    }
    let body = build(&values);

    quote! {
        #[automatically_derived]
        impl #impl_g ::injecta::Injectable<#c> for #self_ty #where_clause {
            const DEPTH: usize = 1 + #depth;
            #[inline]
            // `container` is unused when every field has an explicit value.
            #[allow(unused_variables)]
            fn inject(container: &#c) -> Self {
                #body
            }
        }
    }
}

/// Rejects dependency types that can never be provided by value.
pub(crate) fn check_dependency(ty: &Type) -> syn::Result<()> {
    match ty {
        Type::Reference(_) => Err(syn::Error::new(
            ty.span(),
            "references cannot be injected; take `Arc<T>` (shared) or `T: Clone` (owned) instead",
        )),
        Type::ImplTrait(_) => Err(syn::Error::new(
            ty.span(),
            "`impl Trait` cannot be injected; take `Arc<dyn Trait>` or a generic parameter instead",
        )),
        _ => Ok(()),
    }
}
