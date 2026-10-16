//! `container!`: parses the declaration (a root `struct` plus optional
//! `scope` blocks), validates it with spanned, fix-suggesting errors, and
//! emits the container structs and one `Provide<T>` impl per entry.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::{
    Attribute, Expr, GenericArgument, Ident, PathArguments, Token, Type, Visibility, braced,
    parse::{Parse, ParseStream},
    spanned::Spanned,
};

use crate::derive::combine;
use crate::naming::{dedup, param_name, type_display};

syn::custom_keyword!(scope);

/// The whole `container!` input.
pub(crate) struct Input {
    root: Block,
    scopes: Vec<Block>,
}

struct Block {
    attrs: Vec<Attribute>,
    storage: Option<Type>,
    hooks: Option<Type>,
    vis: Visibility,
    name: Ident,
    entries: Vec<Entry>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Instance,
    Singleton,
    Scoped,
    Transient,
}

struct Entry {
    kind: Kind,
    kind_span: Span,
    ty: Type,
    factory: Option<Expr>,
}

impl Entry {
    fn key(&self) -> String {
        type_display(&self.ty)
    }
}

impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let root = Block::parse_block(input, false)?;
        let mut scopes = Vec::new();
        while !input.is_empty() {
            scopes.push(Block::parse_block(input, true)?);
        }
        let parsed = Self { root, scopes };
        parsed.validate()?;
        Ok(parsed)
    }
}

impl Block {
    fn parse_block(input: ParseStream<'_>, is_scope: bool) -> syn::Result<Self> {
        let mut attrs = Attribute::parse_outer(input)?;
        let vis: Visibility = input.parse()?;
        if is_scope {
            if input.peek(Token![struct]) {
                return Err(input.error("only the first block is a `struct`; declare child containers with `scope Name { .. }`"));
            }
            input.parse::<scope>().map_err(|e| {
                syn::Error::new(
                    e.span(),
                    "expected `scope Name { .. }` after the root container",
                )
            })?;
        } else {
            if input.peek(scope) {
                return Err(input.error("the first block is the root container: `pub struct App { .. }`; scopes follow it"));
            }
            input.parse::<Token![struct]>()?;
        }
        let name: Ident = input.parse()?;
        if input.peek(Token![<]) {
            return Err(
                input.error("containers cannot be generic; register the concrete types instead")
            );
        }
        let content;
        braced!(content in input);
        let mut entries = Vec::new();
        while !content.is_empty() {
            entries.push(content.parse::<Entry>()?);
            if content.is_empty() {
                break;
            }
            content.parse::<Token![,]>()?;
        }

        let (mut storage, mut hooks) = (None, None);
        let mut errors = None;
        attrs.retain(|attr| {
            if !attr.path().is_ident("injecta") {
                return true;
            }
            if is_scope {
                combine(&mut errors, syn::Error::new(attr.span(), "`#[injecta(..)]` options go on the root container; scopes share its storage and hooks"));
                return false;
            }
            let parsed = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("storage") {
                    storage = Some(meta.value()?.parse()?);
                    Ok(())
                } else if meta.path.is_ident("hooks") {
                    hooks = Some(meta.value()?.parse()?);
                    Ok(())
                } else {
                    Err(meta.error("unknown option; expected `storage = Type` or `hooks = Type`"))
                }
            });
            if let Err(err) = parsed {
                combine(&mut errors, err);
            }
            false
        });
        if let Some(err) = errors {
            return Err(err);
        }
        Ok(Self {
            attrs,
            storage,
            hooks,
            vis,
            name,
            entries,
        })
    }
}

impl Parse for Entry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let kind_ident: Ident = input.parse().map_err(|e| {
            syn::Error::new(
                e.span(),
                "expected an entry: `instance T`, `singleton T`, `scoped T` or `transient T`",
            )
        })?;
        let kind = match kind_ident.to_string().as_str() {
            "instance" => Kind::Instance,
            "singleton" => Kind::Singleton,
            "scoped" => Kind::Scoped,
            "transient" => Kind::Transient,
            other => {
                let hint = match other {
                    "factory" | "prototype" => " (`transient` builds a new value on every resolve)",
                    "lazy" | "shared" | "single" => " (`singleton` is created lazily and shared)",
                    "value" | "constant" => " (`instance` takes a value in the constructor)",
                    _ => "",
                };
                return Err(syn::Error::new(
                    kind_ident.span(),
                    format!(
                        "unknown lifetime `{other}`; expected `instance`, `singleton`, `scoped` or `transient`{hint}"
                    ),
                ));
            }
        };
        let ty: Type = input.parse()?;
        let factory = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse::<Expr>()?)
        } else {
            None
        };
        Ok(Self {
            kind,
            kind_span: kind_ident.span(),
            ty,
            factory,
        })
    }
}

impl Input {
    fn validate(&self) -> syn::Result<()> {
        let mut errors = None;
        self.root.validate(None, &mut errors);
        for scope in &self.scopes {
            scope.validate(Some(&self.root), &mut errors);
        }
        errors.map_or(Ok(()), Err)
    }
}

impl Block {
    fn validate(&self, parent: Option<&Block>, errors: &mut Option<syn::Error>) {
        let mut seen: Vec<String> = Vec::new();
        for entry in &self.entries {
            let key = entry.key();
            if seen.contains(&key) {
                combine(
                    errors,
                    syn::Error::new(
                        entry.ty.span(),
                        format!(
                            "`{key}` is registered twice in `{}`; each type has exactly one provider — wrap one of them in a newtype to register two values of the same type",
                            self.name
                        ),
                    ),
                );
            }
            seen.push(key.clone());
            if let Some(parent) = parent {
                if parent.entries.iter().any(|p| p.key() == key) {
                    combine(
                        errors,
                        syn::Error::new(
                            entry.ty.span(),
                            format!(
                                "`{key}` is already provided by `{}`; a scope cannot shadow it — in tests, replace it with `.with(value)`",
                                parent.name
                            ),
                        ),
                    );
                }
            } else if entry.kind == Kind::Scoped {
                combine(
                    errors,
                    syn::Error::new(
                        entry.kind_span,
                        "`scoped` is only valid inside a `scope` block; in the root container use `singleton`",
                    ),
                );
            }
            if entry.kind == Kind::Instance && entry.factory.is_some() {
                combine(
                    errors,
                    syn::Error::new(
                        entry.kind_span,
                        "an `instance` is passed to `new(..)` and has no factory; remove `= ..` or make it a `singleton`",
                    ),
                );
            }
            if let Err(err) = check_type(entry) {
                combine(errors, err);
            }
        }
    }
}

fn check_type(entry: &Entry) -> syn::Result<()> {
    match &entry.ty {
        Type::Reference(_) => Err(syn::Error::new(
            entry.ty.span(),
            "references cannot be registered; register `Arc<T>` to share a value",
        )),
        Type::ImplTrait(_) => Err(syn::Error::new(
            entry.ty.span(),
            "`impl Trait` cannot be registered; register `Arc<dyn Trait>` with a factory",
        )),
        Type::TraitObject(_) => Err(syn::Error::new(
            entry.ty.span(),
            "a trait object needs a pointer: register `Arc<dyn Trait>` (shared) or `Box<dyn Trait>` (transient)",
        )),
        _ if entry.factory.is_none() && entry.kind != Kind::Instance => match wrapper(&entry.ty) {
            Some((_, Type::TraitObject(_))) => Err(syn::Error::new(
                entry.ty.span(),
                "a trait object needs a factory that picks the implementation: `= |c| Arc::new(c.build::<MyImpl>())`",
            )),
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// `Arc<T>` / `Box<T>`: the pointer name and `T`.
fn wrapper(ty: &Type) -> Option<(Ident, &Type)> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    if last.ident != "Arc" && last.ident != "Box" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first() {
        Some(GenericArgument::Type(inner)) if args.args.len() == 1 => {
            Some((last.ident.clone(), inner))
        }
        _ => None,
    }
}

/// Paths the generated code uses, resolved once per invocation.
struct Ctx<'a> {
    root: &'a Block,
    storage: TokenStream,
    hooks: TokenStream,
}

pub(crate) fn expand(input: &Input) -> TokenStream {
    let ctx = Ctx {
        root: &input.root,
        storage: input.root.storage.as_ref().map_or_else(
            || quote!(::injecta::DefaultStorage),
            ToTokens::to_token_stream,
        ),
        hooks: input
            .root
            .hooks
            .as_ref()
            .map_or_else(|| quote!(::injecta::NoHooks), ToTokens::to_token_stream),
    };
    let root = expand_root(&ctx);
    let scopes = input.scopes.iter().map(|scope| expand_scope(&ctx, scope));
    quote!(#root #(#scopes)*)
}

fn field(index: usize) -> Ident {
    format_ident!("__injecta_{}", index)
}

fn info_const(index: usize) -> Ident {
    format_ident!("__INJECTA_INFO_{}", index)
}

fn lifetime_tokens(kind: Kind) -> TokenStream {
    match kind {
        Kind::Instance => quote!(::injecta::Lifetime::Instance),
        Kind::Singleton => quote!(::injecta::Lifetime::Singleton),
        Kind::Scoped => quote!(::injecta::Lifetime::Scoped),
        Kind::Transient => quote!(::injecta::Lifetime::Transient),
    }
}

/// The expression that builds an entry's value in `Self`'s context, and its
/// `DEPTH`. Factories are opaque to the depth check, so they count as 0.
fn constructor(entry: &Entry) -> (TokenStream, TokenStream) {
    let ty = &entry.ty;
    if let Some(factory) = &entry.factory {
        return (
            quote_spanned!(factory.span()=> ::injecta::__private::call_factory::<Self, #ty>(self, #factory)),
            quote!(0usize),
        );
    }
    let (wrap, inner) = match wrapper(ty) {
        Some((pointer, inner)) => (Some(pointer), inner),
        None => (None, ty),
    };
    let inject = quote_spanned!(ty.span()=> <#inner as ::injecta::Injectable<Self>>::inject(self));
    let depth = quote_spanned!(ty.span()=> <#inner as ::injecta::Injectable<Self>>::DEPTH);
    let value = match wrap {
        Some(pointer) if pointer == "Arc" => quote!(::injecta::__private::Arc::new(#inject)),
        Some(_) => quote!(::injecta::__private::Box::new(#inject)),
        None => inject,
    };
    (value, depth)
}

/// `Provide` impl for an entry that this block owns. `hooks` reaches the
/// shared hooks value from `self`.
fn own_provide(
    self_ty: &TokenStream,
    entry: &Entry,
    index: usize,
    hooks: &TokenStream,
) -> TokenStream {
    let ty = &entry.ty;
    let info = info_const(index);
    let field = field(index);
    let resolve_hook = quote!(::injecta::Hooks::on_resolve(#hooks, &Self::#info););
    // Cached entries (instance, singleton, scoped) get `ProvideRef`, and
    // `provide` is a clone of the borrowed value, so the lazy-init code is
    // emitted once per entry.
    let (depth, borrow) = match entry.kind {
        Kind::Instance => (quote!(0usize), quote!(&self.#field)),
        Kind::Singleton | Kind::Scoped => {
            let (value, depth) = constructor(entry);
            (
                depth,
                quote! {
                    ::injecta::SingletonCell::get_or_init(&self.#field, || {
                        let value = #value;
                        ::injecta::Hooks::on_create(#hooks, &Self::#info);
                        value
                    })
                },
            )
        }
        Kind::Transient => {
            let (value, depth) = constructor(entry);
            return quote_spanned! {ty.span()=>
                impl ::injecta::Provide<#ty> for #self_ty {
                    const DEPTH: usize = #depth;
                    #[inline]
                    fn provide(&self) -> #ty {
                        #resolve_hook
                        #value
                    }
                }
            };
        }
    };
    quote_spanned! {ty.span()=>
        impl ::injecta::Provide<#ty> for #self_ty {
            const DEPTH: usize = #depth;
            #[inline]
            // `Copy` entries are cloned too: one code path for every type.
            #[allow(clippy::clone_on_copy)]
            fn provide(&self) -> #ty {
                ::injecta::__private::Clone::clone(::injecta::ProvideRef::<#ty>::provide_ref(self))
            }
        }

        impl ::injecta::ProvideRef<#ty> for #self_ty {
            #[inline]
            fn provide_ref(&self) -> &#ty {
                #resolve_hook
                #borrow
            }
        }
    }
}

struct Layout {
    fields: Vec<TokenStream>,
    params: Vec<TokenStream>,
    inits: Vec<TokenStream>,
    infos: Vec<TokenStream>,
    info_names: Vec<Ident>,
    overrides: Vec<TokenStream>,
}

fn layout(ctx: &Ctx<'_>, block: &Block, self_ty: &TokenStream) -> Layout {
    let storage = &ctx.storage;
    let names = dedup(
        block
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Instance)
            .map(|e| param_name(&e.ty))
            .collect(),
    );
    let mut names = names.into_iter();
    let mut out = Layout {
        fields: vec![],
        params: vec![],
        inits: vec![],
        infos: vec![],
        info_names: vec![],
        overrides: vec![],
    };
    for (index, entry) in block.entries.iter().enumerate() {
        let ty = &entry.ty;
        let field = field(index);
        let info = info_const(index);
        let lifetime = lifetime_tokens(entry.kind);
        let type_name = type_display(ty);
        let has_factory = entry.factory.is_some();
        out.infos.push(quote! {
            const #info: ::injecta::ProviderInfo = ::injecta::ProviderInfo { type_name: #type_name, lifetime: #lifetime, factory: #has_factory };
        });
        out.info_names.push(info);
        match entry.kind {
            Kind::Instance => {
                let param = Ident::new(&names.next().unwrap_or_default(), ty.span());
                out.fields.push(quote!(#field: #ty));
                out.params.push(quote!(#param: #ty));
                out.inits.push(quote!(#field: #param));
            }
            Kind::Singleton | Kind::Scoped => {
                out.fields
                    .push(quote!(#field: <#storage as ::injecta::Storage>::Cell<#ty>));
                out.inits
                    .push(quote!(#field: ::injecta::SingletonCell::empty()));
                out.overrides.push(quote! {
                    impl ::injecta::Override<#ty> for #self_ty {
                        #[inline]
                        fn set(&mut self, value: #ty) {
                            self.#field = ::injecta::SingletonCell::preset(value);
                        }
                    }
                });
            }
            Kind::Transient => {}
        }
    }
    out
}

fn depth_checks(self_ty: &TokenStream, name: &Ident, entries: &[&Type]) -> TokenStream {
    let checks = entries.iter().map(|ty| {
        let message = format!(
            "the dependency chain of `{}` in `{name}` is longer than injecta::MAX_DEPTH",
            type_display(ty)
        );
        quote_spanned! {ty.span()=>
            ::core::assert!(::injecta::__private::within_max_depth(<#self_ty as ::injecta::Provide<#ty>>::DEPTH), #message);
        }
    });
    // Evaluating every DEPTH here is what turns a dependency cycle into a
    // compile error (rustc reports the const-evaluation cycle).
    quote!(const _: () = { #(#checks)* };)
}

fn common_impls(
    name: &Ident,
    self_ty: &TokenStream,
    impl_g: &TokenStream,
    l: &Layout,
) -> TokenStream {
    let infos = &l.infos;
    let info_names = &l.info_names;
    let name_str = name.to_string();
    quote! {
        #[allow(non_upper_case_globals)]
        impl #impl_g #self_ty {
            #(#infos)*
        }

        impl #impl_g ::injecta::Container for #self_ty {
            const NAME: &'static str = #name_str;
            const PROVIDERS: &'static [::injecta::ProviderInfo] = &[#(Self::#info_names),*];
        }

        impl #impl_g ::core::fmt::Debug for #self_ty {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&<Self as ::injecta::Container>::describe(), f)
            }
        }
    }
}

fn expand_root(ctx: &Ctx<'_>) -> TokenStream {
    let block = ctx.root;
    let Block {
        attrs, vis, name, ..
    } = block;
    let hooks_ty = &ctx.hooks;
    let self_ty = quote!(#name);
    let l = layout(ctx, block, &self_ty);
    let (fields, params, inits, overrides) = (&l.fields, &l.params, &l.inits, &l.overrides);
    let hooks = quote!(&self.__injecta_hooks);
    let provides = block
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| own_provide(&self_ty, e, i, &hooks));
    let common = common_impls(name, &self_ty, &quote!(), &l);
    let types: Vec<&Type> = block.entries.iter().map(|e| &e.ty).collect();
    let checks = depth_checks(&self_ty, name, &types);
    quote! {
        #(#attrs)*
        #vis struct #name {
            __injecta_hooks: #hooks_ty,
            #(#fields,)*
        }

        impl #name {
            /// Creates the container. Arguments are its `instance` entries in
            /// declaration order; singletons are created on first resolve.
            #[allow(clippy::new_without_default, clippy::too_many_arguments)]
            #[must_use]
            #vis fn new(#(#params),*) -> Self {
                Self {
                    __injecta_hooks: ::injecta::__private::Default::default(),
                    #(#inits,)*
                }
            }

            /// Replaces a `singleton` with `value` (typically a test double).
            #[must_use]
            #vis fn with<T>(mut self, value: T) -> Self
            where
                Self: ::injecta::Override<T>,
            {
                ::injecta::Override::set(&mut self, value);
                self
            }

            /// The container's hooks.
            #vis fn hooks(&self) -> &#hooks_ty {
                &self.__injecta_hooks
            }
        }

        #common
        #(#overrides)*
        #(#provides)*
        #checks
    }
}

fn expand_scope(ctx: &Ctx<'_>, block: &Block) -> TokenStream {
    let root = ctx.root;
    let root_name = &root.name;
    let Block {
        attrs, vis, name, ..
    } = block;
    let self_ty = quote!(#name<'_>);
    let mut l = layout(ctx, block, &self_ty);
    let (fields, params, inits) = (&l.fields, &l.params, &l.inits);
    let hooks = quote!(&self.__injecta_parent.__injecta_hooks);
    let own = block
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| own_provide(&self_ty, e, i, &hooks));

    // Root instances and singletons are shared with the parent; root
    // transients are rebuilt here so they can see this scope's values.
    let inherited = root.entries.iter().enumerate().map(|(index, entry)| {
        let ty = &entry.ty;
        let info = info_const(index);
        if entry.kind == Kind::Transient {
            let (value, depth) = constructor(entry);
            quote_spanned! {ty.span()=>
                impl ::injecta::Provide<#ty> for #self_ty {
                    const DEPTH: usize = #depth;
                    #[inline]
                    fn provide(&self) -> #ty {
                        ::injecta::Hooks::on_resolve(#hooks, &#root_name::#info);
                        #value
                    }
                }
            }
        } else {
            quote_spanned! {ty.span()=>
                impl ::injecta::Provide<#ty> for #self_ty {
                    const DEPTH: usize = <#root_name as ::injecta::Provide<#ty>>::DEPTH;
                    #[inline]
                    fn provide(&self) -> #ty {
                        <#root_name as ::injecta::Provide<#ty>>::provide(self.__injecta_parent)
                    }
                }

                impl ::injecta::ProvideRef<#ty> for #self_ty {
                    #[inline]
                    fn provide_ref(&self) -> &#ty {
                        <#root_name as ::injecta::ProvideRef<#ty>>::provide_ref(self.__injecta_parent)
                    }
                }
            }
        }
    });
    let overrides = std::mem::take(&mut l.overrides);
    let common = common_impls(name, &quote!(#name<'p>), &quote!(<'p>), &l);
    let types: Vec<&Type> = root
        .entries
        .iter()
        .chain(&block.entries)
        .map(|e| &e.ty)
        .collect();
    let checks = depth_checks(&quote!(#name<'static>), name, &types);
    quote! {
        #(#attrs)*
        #vis struct #name<'p> {
            __injecta_parent: &'p #root_name,
            #(#fields,)*
        }

        impl<'p> #name<'p> {
            /// Opens a scope of `parent`. Arguments are the scope's `instance`
            /// entries in declaration order.
            #[allow(clippy::too_many_arguments)]
            #[must_use]
            #vis fn new(parent: &'p #root_name, #(#params),*) -> Self {
                Self {
                    __injecta_parent: parent,
                    #(#inits,)*
                }
            }

            /// Replaces a `scoped` entry with `value` (typically a test double).
            #[must_use]
            #vis fn with<T>(mut self, value: T) -> Self
            where
                Self: ::injecta::Override<T>,
            {
                ::injecta::Override::set(&mut self, value);
                self
            }

            /// The container this scope was opened from.
            #vis fn parent(&self) -> &'p #root_name {
                self.__injecta_parent
            }
        }

        #common
        #(#overrides)*
        #(#own)*
        #(#inherited)*
        #checks
    }
}

#[cfg(test)]
mod tests {
    use super::Input;

    fn error_of(src: &str) -> String {
        match syn::parse_str::<Input>(src) {
            Ok(_) => String::from("<parsed>"),
            Err(err) => err
                .into_iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    #[test]
    fn unknown_lifetime_suggests_the_closest_keyword() {
        let err = error_of("struct App { lazy Db }");
        assert!(err.contains("unknown lifetime `lazy`"), "{err}");
        assert!(err.contains("`singleton` is created lazily"), "{err}");
    }

    #[test]
    fn duplicate_type_suggests_newtype() {
        let err = error_of("struct App { instance Config, singleton Config }");
        assert!(
            err.contains("registered twice") && err.contains("newtype"),
            "{err}"
        );
    }

    #[test]
    fn scoped_in_root_points_to_singleton() {
        assert!(
            error_of("struct App { scoped Arc<Db> }")
                .contains("in the root container use `singleton`")
        );
    }

    #[test]
    fn scope_cannot_shadow_parent_entry() {
        let err = error_of("struct App { singleton Arc<Db> } scope Req { scoped Arc<Db> }");
        assert!(err.contains("already provided by `App`"), "{err}");
    }

    #[test]
    fn trait_object_without_factory_shows_factory_example() {
        let err = error_of("struct App { singleton Arc<dyn Log> }");
        assert!(err.contains("c.build::<MyImpl>()"), "{err}");
    }

    #[test]
    fn instance_with_factory_is_rejected() {
        assert!(error_of("struct App { instance Config = |_| Config }").contains("has no factory"));
    }

    #[test]
    fn all_errors_are_reported_at_once() {
        let err = error_of("struct App { scoped A, instance B = |_| B, singleton &C }");
        assert_eq!(err.lines().count(), 3, "{err}");
    }

    #[test]
    fn valid_declaration_parses() {
        let src = "#[injecta(storage = SingleThread)] pub struct App { instance Config, singleton Arc<Db>, \
                   singleton Arc<dyn Log> = |c| Arc::new(c.build::<Stdout>()), transient Repo, } \
                   pub scope Req { instance UserId, scoped Arc<Session>, transient Handler }";
        assert_eq!(error_of(src), "<parsed>");
    }
}
