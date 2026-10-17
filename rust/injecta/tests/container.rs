//! End-to-end behaviour of generated containers: lifetimes, scopes,
//! overrides, factories, hooks, storage and introspection.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use injecta::{Container, Hooks, Injectable, Lifetime, Provide, ProviderInfo, Resolve};

#[derive(Clone, Debug, PartialEq)]
struct Config {
    name: &'static str,
}

trait Greeter: Send + Sync {
    fn greet(&self) -> String;
}

#[derive(Injectable)]
struct English {
    config: Config,
}

impl Greeter for English {
    fn greet(&self) -> String {
        format!("hello {}", self.config.name)
    }
}

struct Fake;

impl Greeter for Fake {
    fn greet(&self) -> String {
        "fake".into()
    }
}

#[derive(Injectable)]
struct Database {
    config: Config,
}

#[derive(Injectable)]
struct Repo {
    db: Arc<Database>,
    greeter: Arc<dyn Greeter>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct UserId(u64);

#[derive(Injectable)]
struct Session {
    user: UserId,
}

#[derive(Injectable)]
struct Handler {
    repo: Repo,
    session: Arc<Session>,
}

injecta::container! {
    /// The test application.
    pub struct App {
        instance Config,
        singleton Arc<dyn Greeter> = |c| Arc::new(c.build::<English>()),
        singleton Arc<Database>,
        transient Repo,
    }
    /// One request.
    pub scope Request {
        instance UserId,
        scoped Arc<Session>,
        transient Handler,
    }
}

fn app() -> App {
    App::new(Config { name: "ivan" })
}

#[test]
fn instance_resolves_to_a_clone_of_the_constructor_argument() {
    assert_eq!(app().resolve::<Config>(), Config { name: "ivan" });
}

#[test]
fn singleton_is_created_once_and_shared() {
    let app = app();
    assert!(Arc::ptr_eq(
        &app.resolve::<Arc<Database>>(),
        &app.resolve::<Arc<Database>>()
    ));
}

#[test]
fn transients_share_singleton_dependencies() {
    let app = app();
    let (a, b) = (app.resolve::<Repo>(), app.resolve::<Repo>());
    assert!(Arc::ptr_eq(&a.db, &b.db));
    assert_eq!(a.db.config.name, "ivan");
}

#[test]
fn factory_binds_trait_object_to_implementation() {
    assert_eq!(app().resolve::<Arc<dyn Greeter>>().greet(), "hello ivan");
}

#[test]
fn override_replaces_singleton_seen_by_dependents() {
    let app = app().with(Arc::new(Fake) as Arc<dyn Greeter>);
    assert_eq!(app.resolve::<Repo>().greeter.greet(), "fake");
}

#[test]
fn scope_caches_scoped_values_per_scope() {
    let app = app();
    let first = Request::new(&app, UserId(1));
    let second = Request::new(&app, UserId(2));
    let (a, b) = (first.resolve::<Handler>(), first.resolve::<Handler>());
    assert!(Arc::ptr_eq(&a.session, &b.session));
    assert!(Arc::ptr_eq(&a.repo.db, &b.repo.db));
    assert_eq!(second.resolve::<Handler>().session.user, UserId(2));
}

#[test]
fn scope_shares_root_singletons() {
    let app = app();
    let request = Request::new(&app, UserId(1));
    assert!(Arc::ptr_eq(
        &request.resolve::<Arc<Database>>(),
        &app.resolve::<Arc<Database>>()
    ));
    assert_eq!(
        request.parent().resolve::<Config>(),
        app.resolve::<Config>()
    );
}

#[test]
fn scope_override_replaces_scoped_value() {
    let app = app();
    let request = Request::new(&app, UserId(1)).with(Arc::new(Session { user: UserId(99) }));
    assert_eq!(request.resolve::<Handler>().session.user, UserId(99));
}

#[test]
fn depth_counts_the_longest_chain() {
    assert_eq!(<App as Provide<Config>>::DEPTH, 0);
    assert_eq!(<App as Provide<Arc<Database>>>::DEPTH, 1);
    assert_eq!(<App as Provide<Repo>>::DEPTH, 2);
    assert_eq!(<Request<'static> as Provide<Handler>>::DEPTH, 3);
}

#[test]
fn describe_lists_entries_in_declaration_order() {
    assert_eq!(
        App::describe().to_string(),
        "App\n  instance  Config\n  singleton Arc<dyn Greeter> (factory)\n  singleton Arc<Database>\n  transient Repo\n"
    );
    assert_eq!(
        Request::PROVIDERS[1],
        ProviderInfo {
            type_name: "Arc<Session>",
            lifetime: Lifetime::Scoped,
            factory: false
        }
    );
    assert_eq!(format!("{:?}", app()), App::describe().to_string());
}

#[test]
fn thread_safe_container_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<App>();
    assert_send_sync::<Request<'static>>();
}

mod hooks_and_storage {
    use super::{AtomicUsize, Hooks, Injectable, Ordering, ProviderInfo, Resolve};

    #[derive(Default)]
    struct Counting {
        created: AtomicUsize,
        resolved: AtomicUsize,
    }

    impl Hooks for Counting {
        fn on_create(&self, _info: &ProviderInfo) {
            self.created.fetch_add(1, Ordering::Relaxed);
        }
        fn on_resolve(&self, _info: &ProviderInfo) {
            self.resolved.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[derive(Clone, Injectable)]
    struct Cache;

    #[derive(Injectable)]
    struct User {
        _cache: Cache,
    }

    injecta::container! {
        #[injecta(storage = injecta::SingleThread, hooks = Counting)]
        struct Local {
            singleton Cache,
            transient User,
        }
    }

    #[test]
    fn hooks_see_every_resolve_and_one_creation() {
        let local = Local::new();
        local.resolve::<User>();
        local.resolve::<User>();
        assert_eq!(local.hooks().created.load(Ordering::Relaxed), 1);
        // 2 x User + 2 x Cache (resolved as User's dependency)
        assert_eq!(local.hooks().resolved.load(Ordering::Relaxed), 4);
    }
}

mod derive_shapes {
    use super::{Config, Injectable, Resolve};

    #[derive(Injectable)]
    struct Unit;

    #[derive(Injectable)]
    struct Tuple(Config, #[inject(value = 7)] u8);

    #[derive(Injectable)]
    struct Generic<T> {
        inner: T,
        #[inject(default)]
        count: usize,
    }

    #[derive(Injectable)]
    struct Boxed {
        _unit: Unit,
    }

    injecta::container! {
        struct Shapes {
            instance Config,
            transient Unit,
            transient Tuple,
            transient Generic<Config>,
            transient Box<Boxed>,
        }
    }

    #[test]
    fn tuple_unit_generic_and_boxed_structs_are_injectable() {
        let shapes = Shapes::new(Config { name: "x" });
        let tuple = shapes.resolve::<Tuple>();
        assert_eq!((tuple.0.name, tuple.1), ("x", 7));
        let generic = shapes.resolve::<Generic<Config>>();
        assert_eq!((generic.inner.name, generic.count), ("x", 0));
        let _: Box<Boxed> = shapes.resolve();
        let _: Unit = shapes.resolve();
    }
}
