//! Parsing of the macro arguments and the `#[plugwright(...)]` helper attribute.

use proc_macro2::TokenStream;
use syn::{Attribute, Ident, LitStr, Path, meta::ParseNestedMeta};

/// Arguments of `#[plugin(...)]` / `#[interface(...)]`.
#[derive(Default)]
pub struct Args {
    /// `crate = path`: where the runtime crate lives when it is renamed.
    pub krate: Option<Path>,
    /// `no_export` (plugin only): implement `Plugin` but skip `export_plugin!`.
    pub no_export: bool,
    /// `client = Name` (interface only): name of the generated client struct.
    pub client: Option<Ident>,
}

impl Args {
    pub fn parse(tokens: TokenStream, allowed: &[&str]) -> syn::Result<Self> {
        let mut args = Args::default();
        let parser = syn::meta::parser(|meta| {
            let key = key(&meta, allowed)?;
            match key.as_str() {
                "crate" => args.krate = Some(meta.value()?.parse()?),
                "no_export" => args.no_export = true,
                "client" => args.client = Some(meta.value()?.parse()?),
                _ => unreachable!("filtered by `key`"),
            }
            Ok(())
        });
        syn::parse::Parser::parse2(parser, tokens)?;
        Ok(args)
    }
}

/// `#[plugwright(...)]` on a method.
#[derive(Default)]
pub struct FnAttrs {
    pub skip: bool,
    pub init: bool,
    pub name: Option<LitStr>,
}

/// Remove every `#[plugwright(...)]` from `attrs` and parse it.
pub fn take(attrs: &mut Vec<Attribute>, allowed: &[&str]) -> syn::Result<FnAttrs> {
    let mut out = FnAttrs::default();
    let mut err: Option<syn::Error> = None;
    attrs.retain(|attr| {
        if !attr.path().is_ident("plugwright") {
            return true;
        }
        let res = attr.parse_nested_meta(|meta| {
            match key(&meta, allowed)?.as_str() {
                "skip" => out.skip = true,
                "init" => out.init = true,
                "name" => out.name = Some(meta.value()?.parse()?),
                _ => unreachable!("filtered by `key`"),
            }
            Ok(())
        });
        if let Err(e) = res {
            match &mut err {
                Some(acc) => acc.combine(e),
                None => err = Some(e),
            }
        }
        false
    });
    err.map_or(Ok(out), Err)
}

fn key(meta: &ParseNestedMeta<'_>, allowed: &[&str]) -> syn::Result<String> {
    let key = meta.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
    if allowed.contains(&key.as_str()) {
        Ok(key)
    } else {
        Err(meta.error(format!("unknown argument; expected one of: {}", allowed.join(", "))))
    }
}
