//! `#[derive(Injectable)]`: struct fields become dependencies. Field-level
//! `#[inject(...)]` options are parsed here; the impl itself comes from `deps`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, spanned::Spanned};

use crate::deps::{Source, check_dependency, injectable_impl};

pub(crate) fn expand(input: &DeriveInput) -> TokenStream {
    match try_expand(input) {
        Ok(tokens) => tokens,
        Err(err) => err.to_compile_error(),
    }
}

fn try_expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "`Injectable` can be derived for structs only; for an enum, put `#[injecta::injectable]` on an `impl` block with an `#[inject]` constructor",
        ));
    };
    let mut errors: Option<syn::Error> = None;
    let mut sources = Vec::with_capacity(data.fields.len());
    for field in &data.fields {
        match field_source(field) {
            Ok(source) => sources.push(source),
            Err(err) => combine(&mut errors, err),
        }
    }
    if let Some(err) = errors {
        return Err(err);
    }

    let ident = &input.ident;
    let (_, ty_g, _) = input.generics.split_for_impl();
    let self_ty = quote!(#ident #ty_g);
    let fields = &data.fields;
    Ok(injectable_impl(
        &self_ty,
        &input.generics,
        &sources,
        |values| match fields {
            Fields::Named(named) => {
                let idents = named.named.iter().map(|f| &f.ident);
                quote!(Self { #(#idents: #values),* })
            }
            Fields::Unnamed(_) => quote!(Self(#(#values),*)),
            Fields::Unit => quote!(Self),
        },
    ))
}

fn field_source(field: &Field) -> syn::Result<Source> {
    let mut source = None;
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("inject")) {
        attr.parse_nested_meta(|meta| {
            if source.is_some() {
                return Err(meta.error("a field takes one `#[inject(...)]` option"));
            }
            if meta.path.is_ident("default") {
                source = Some(Source::Expr(quote!(
                    ::injecta::__private::Default::default()
                )));
                Ok(())
            } else if meta.path.is_ident("value") {
                let expr: syn::Expr = meta.value()?.parse()?;
                source = Some(Source::Expr(quote!(#expr)));
                Ok(())
            } else {
                Err(meta.error(
                    "unknown option; expected `#[inject(default)]` or `#[inject(value = expr)]`",
                ))
            }
        })?;
    }
    if let Some(source) = source {
        return Ok(source);
    }
    check_dependency(&field.ty).map_err(|err| {
        syn::Error::new(
            field.ty.span(),
            format!("{err}, or give the field a fixed value with `#[inject(value = ...)]`"),
        )
    })?;
    Ok(Source::Resolve(field.ty.clone()))
}

pub(crate) fn combine(slot: &mut Option<syn::Error>, err: syn::Error) {
    match slot {
        Some(existing) => existing.combine(err),
        None => *slot = Some(err),
    }
}

#[cfg(test)]
mod tests {
    use super::try_expand;

    fn error_of(src: &str) -> String {
        let input = syn::parse_str(src).unwrap();
        try_expand(&input).unwrap_err().to_string()
    }

    #[test]
    fn enum_is_rejected_with_constructor_hint() {
        assert!(error_of("enum E { A }").contains("#[injecta::injectable]"));
    }

    #[test]
    fn reference_field_is_rejected_with_arc_hint() {
        assert!(error_of("struct S<'a> { db: &'a Db }").contains("take `Arc<T>`"));
    }

    #[test]
    fn unknown_field_option_is_rejected() {
        assert!(error_of("struct S { #[inject(lazy)] db: Db }").contains("unknown option"));
    }

    #[test]
    fn generated_impl_bounds_every_resolved_field() {
        let input = syn::parse_str("struct S<T> { a: A, b: T, #[inject(default)] n: u8 }").unwrap();
        let out = try_expand(&input).unwrap().to_string();
        assert!(out.contains(":: injecta :: Provide < A >"));
        assert!(out.contains(":: injecta :: Provide < T >"));
        assert!(!out.contains("Provide < u8 >"));
    }
}
