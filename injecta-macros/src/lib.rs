//! Procedural macros for `injecta`. The runtime crate re-exports them; depend
//! on `injecta`, not on this crate. Each macro lives in its own module; the
//! code they share (dependency bounds, depth constants) is in `deps`.

mod container;
mod deps;
mod derive;
mod injectable;
mod naming;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Declares a container and, optionally, its scopes.
///
/// ```
/// use std::sync::Arc;
/// use injecta::{Injectable, Resolve};
///
/// #[derive(Clone)]
/// struct Config { url: &'static str }
///
/// #[derive(Injectable)]
/// struct Db { config: Config }
///
/// #[derive(Injectable)]
/// struct Repo { db: Arc<Db> }
///
/// injecta::container! {
///     pub struct App {
///         instance Config,
///         singleton Arc<Db>,
///         transient Repo,
///     }
/// }
///
/// let app = App::new(Config { url: "postgres://" });
/// let repo = app.resolve::<Repo>();
/// assert_eq!(repo.db.config.url, "postgres://");
/// ```
///
/// The full syntax (lifetimes, factories, scopes, options) is in the
/// `injecta` crate guide.
#[proc_macro]
pub fn container(input: TokenStream) -> TokenStream {
    container::expand(&parse_macro_input!(input as container::Input)).into()
}

/// Implements `injecta::Injectable` for a struct: every field is a
/// dependency resolved from the container.
///
/// Field options: `#[inject(default)]` uses `Default::default()`,
/// `#[inject(value = expr)]` uses `expr`; neither is resolved.
///
/// ```
/// use std::sync::Arc;
/// use injecta::{Injectable, Resolve};
///
/// #[derive(Clone)]
/// struct Config;
///
/// #[derive(Injectable)]
/// struct Service {
///     config: Config,
///     #[inject(value = 3)]
///     retries: u32,
/// }
///
/// injecta::container! {
///     struct App { instance Config, transient Service }
/// }
///
/// assert_eq!(App::new(Config).resolve::<Service>().retries, 3);
/// ```
#[proc_macro_derive(Injectable, attributes(inject))]
pub fn derive_injectable(input: TokenStream) -> TokenStream {
    derive::expand(&parse_macro_input!(input as syn::DeriveInput)).into()
}

/// Implements `injecta::Injectable` from the constructor marked `#[inject]`
/// in an inherent `impl` block. Its parameters are the dependencies.
///
/// ```
/// use injecta::Resolve;
///
/// #[derive(Clone)]
/// struct Config { pool: u32 }
///
/// struct Pool { size: u32 }
///
/// #[injecta::injectable]
/// impl Pool {
///     #[inject]
///     fn new(config: Config) -> Self {
///         Self { size: config.pool * 2 }
///     }
/// }
///
/// injecta::container! {
///     struct App { instance Config, transient Pool }
/// }
///
/// assert_eq!(App::new(Config { pool: 4 }).resolve::<Pool>().size, 8);
/// ```
#[proc_macro_attribute]
pub fn injectable(attr: TokenStream, item: TokenStream) -> TokenStream {
    injectable::expand(&attr.into(), &parse_macro_input!(item as syn::ItemImpl)).into()
}
