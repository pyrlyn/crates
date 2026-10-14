//! Caching strategies and the borrow fast path: lazy singletons built
//! exactly once (`ThreadSafe` under contention, `SingleThread`), `get`
//! borrowing without cloning, fresh transients, per-scope values and their
//! drop, overrides that skip construction, and constructor injection.
//!
//! Not tested here: a factory that resolves its own entry recurses (stack
//! overflow with `SingleThread`) or deadlocks (`ThreadSafe`); a runtime guard
//! is task P0-3 in `todo.md`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use injecta::{Injectable, Provide, ProvideRef, Resolve, SingleThread, ThreadSafe};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Port(u16);

/// Counts constructions per type; each test uses its own container type and
/// its own counter, so tests running in parallel do not interfere.
macro_rules! counted {
    ($name:ident, $counter:ident) => {
        static $counter: AtomicUsize = AtomicUsize::new(0);

        #[allow(dead_code)] // not every test reads both fields
        struct $name {
            serial: usize,
            port: Port,
        }

        #[injecta::injectable]
        impl $name {
            #[inject]
            fn new(port: Port) -> Self {
                Self {
                    serial: $counter.fetch_add(1, Ordering::SeqCst) + 1,
                    port,
                }
            }
        }
    };
}

mod lazy_singletons {
    use super::{Arc, AtomicUsize, Ordering, Port, Resolve};

    counted!(SharedDb, SHARED_DB);
    counted!(LocalDb, LOCAL_DB);

    injecta::container! {
        struct Shared {
            instance Port,
            singleton Arc<SharedDb>,
        }
    }

    injecta::container! {
        #[injecta(storage = injecta::SingleThread)]
        struct Local {
            instance Port,
            singleton Arc<LocalDb>,
        }
    }

    #[test]
    fn thread_safe_singleton_is_lazy_and_built_once() {
        let app = Shared::new(Port(1));
        assert_eq!(
            SHARED_DB.load(Ordering::SeqCst),
            0,
            "built before first resolve"
        );
        let first = app.resolve::<Arc<SharedDb>>();
        let second = app.resolve::<Arc<SharedDb>>();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!((first.serial, first.port), (1, Port(1)));
        assert_eq!(SHARED_DB.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn single_thread_singleton_is_lazy_and_built_once() {
        let app = Local::new(Port(2));
        assert_eq!(
            LOCAL_DB.load(Ordering::SeqCst),
            0,
            "built before first resolve"
        );
        let first = app.get::<Arc<LocalDb>>().serial;
        let again = app.resolve::<Arc<LocalDb>>();
        assert_eq!((first, again.serial, again.port), (1, 1, Port(2)));
        assert_eq!(LOCAL_DB.load(Ordering::SeqCst), 1);
    }
}

mod contention {
    use super::{Arc, AtomicUsize, Ordering, Port, Resolve};

    counted!(Pool, POOL);

    injecta::container! {
        struct App {
            instance Port,
            singleton Arc<Pool>,
        }
    }

    #[test]
    fn thread_safe_singleton_is_built_once_under_contention() {
        let app = App::new(Port(3));
        let barrier = std::sync::Barrier::new(8);
        let pools: Vec<Arc<Pool>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        barrier.wait();
                        app.resolve::<Arc<Pool>>()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(POOL.load(Ordering::SeqCst), 1);
        assert!(pools.iter().all(|p| Arc::ptr_eq(p, &pools[0])));
        let borrowed: &Arc<Pool> = app.get();
        assert!(Arc::ptr_eq(borrowed, &pools[0]));
    }

    #[test]
    fn thread_safe_containers_and_scopes_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<App>();
    }
}

mod borrowing {
    use super::{Arc, Port, Provide, ProvideRef, Resolve};

    #[derive(injecta::Injectable)]
    struct Db {
        port: Port,
    }

    injecta::container! {
        struct App {
            instance Port,
            singleton Arc<Db>,
        }
        scope Request {
            instance u64,
            scoped Arc<String> = |c| Arc::new(format!("request {}", c.resolve::<u64>())),
        }
    }

    #[test]
    fn get_borrows_the_cached_singleton_without_touching_the_count() {
        let app = App::new(Port(4));
        let owned = app.resolve::<Arc<Db>>();
        let before = Arc::strong_count(&owned);
        let borrowed = app.get::<Arc<Db>>();
        assert!(Arc::ptr_eq(borrowed, &owned));
        assert_eq!(Arc::strong_count(&owned), before);
        assert_eq!(borrowed.port, Port(4));
    }

    #[test]
    fn get_borrows_instances() {
        let app = App::new(Port(5));
        assert_eq!(*app.get::<Port>(), Port(5));
        assert!(std::ptr::eq(app.get::<Port>(), app.get::<Port>()));
    }

    #[test]
    fn get_works_for_scoped_and_inherited_entries() {
        let app = App::new(Port(6));
        let request = Request::new(&app, 9);
        assert_eq!(request.get::<Arc<String>>().as_str(), "request 9");
        assert!(Arc::ptr_eq(
            request.get::<Arc<String>>(),
            &request.resolve::<Arc<String>>()
        ));
        assert!(Arc::ptr_eq(request.get::<Arc<Db>>(), app.get::<Arc<Db>>()));
        assert_eq!(*request.get::<Port>(), Port(6));
        assert_eq!(*request.get::<u64>(), 9);
    }

    fn port_of<C: ProvideRef<Port>>(container: &C) -> u16 {
        container.get::<Port>().0
    }

    fn owned_port_of<C: Provide<Port>>(container: &C) -> u16 {
        container.resolve::<Port>().0
    }

    #[test]
    fn generic_code_can_require_borrowable_entries() {
        let app = App::new(Port(7));
        assert_eq!(port_of(&app), 7);
        assert_eq!(port_of(&Request::new(&app, 1)), 7);
        assert_eq!(owned_port_of(&app), 7);
    }
}

mod transients {
    use super::{AtomicUsize, Ordering, Port, Resolve};

    counted!(Job, JOB);

    injecta::container! {
        struct App {
            instance Port,
            transient Job,
        }
    }

    #[test]
    fn transients_are_built_fresh_on_every_resolve() {
        let app = App::new(Port(8));
        let a = app.resolve::<Job>();
        let b = app.resolve::<Job>();
        assert_ne!(a.serial, b.serial);
        assert_eq!(JOB.load(Ordering::SeqCst), 2);
        assert_eq!(a.port, Port(8));
    }
}

mod scopes {
    use super::{Arc, AtomicUsize, Ordering, Port, Resolve};

    static DROPPED: AtomicUsize = AtomicUsize::new(0);

    #[derive(injecta::Injectable)]
    struct Session {
        user: u64,
    }

    impl Drop for Session {
        fn drop(&mut self) {
            DROPPED.fetch_add(1, Ordering::SeqCst);
        }
    }

    counted!(Db, DB);

    injecta::container! {
        struct App {
            instance Port,
            singleton Arc<Db>,
        }
        scope Request {
            instance u64,
            scoped Arc<Session>,
        }
    }

    #[test]
    fn scoped_values_are_per_scope_and_dropped_with_it() {
        let app = App::new(Port(9));
        {
            let one = Request::new(&app, 1);
            let two = Request::new(&app, 2);
            let a = one.resolve::<Arc<Session>>();
            assert!(Arc::ptr_eq(&a, &one.resolve::<Arc<Session>>()));
            let b = two.resolve::<Arc<Session>>();
            assert!(!Arc::ptr_eq(&a, &b));
            assert_eq!((a.user, b.user), (1, 2));
            assert!(Arc::ptr_eq(
                &one.resolve::<Arc<Db>>(),
                &two.resolve::<Arc<Db>>()
            ));
        }
        assert_eq!(DROPPED.load(Ordering::SeqCst), 2);
        assert_eq!(DB.load(Ordering::SeqCst), 1);
    }
}

mod overrides {
    use super::{Arc, AtomicUsize, Ordering, Port, Resolve};

    counted!(RealDb, REAL_DB);
    counted!(LocalDb, LOCAL_DB);

    injecta::container! {
        struct App {
            instance Port,
            singleton Arc<RealDb>,
        }
        scope Request {
            scoped Arc<String> = |_| Arc::new(String::from("real")),
        }
    }

    injecta::container! {
        #[injecta(storage = injecta::SingleThread)]
        struct Local {
            instance Port,
            singleton Arc<LocalDb>,
        }
    }

    #[test]
    fn override_skips_the_constructor() {
        let fake = Arc::new(RealDb {
            serial: 99,
            port: Port(0),
        });
        let app = App::new(Port(10)).with(Arc::clone(&fake));
        assert!(Arc::ptr_eq(&app.resolve::<Arc<RealDb>>(), &fake));
        assert_eq!(app.get::<Arc<RealDb>>().serial, 99);
        assert_eq!(REAL_DB.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn override_works_with_single_thread_storage() {
        let app = Local::new(Port(11)).with(Arc::new(LocalDb {
            serial: 7,
            port: Port(0),
        }));
        assert_eq!(app.resolve::<Arc<LocalDb>>().serial, 7);
        assert_eq!(LOCAL_DB.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn scope_override_is_local_to_that_scope() {
        let app = App::new(Port(12));
        let faked = Request::new(&app).with(Arc::new(String::from("fake")));
        let real = Request::new(&app);
        assert_eq!(faked.get::<Arc<String>>().as_str(), "fake");
        assert_eq!(real.get::<Arc<String>>().as_str(), "real");
    }
}

mod constructor_injection {
    use super::{Arc, Injectable, Port, Resolve};

    #[derive(Clone, Debug, PartialEq)]
    struct Url(&'static str);

    #[derive(Injectable)]
    struct Client {
        url: Url,
        port: Port,
        #[inject(default)]
        retries: u8,
        #[inject(value = Vec::from([1, 2]))]
        backoff: Vec<u32>,
    }

    struct Service {
        label: String,
        url: Url,
        client: Arc<Client>,
    }

    #[injecta::injectable]
    impl Service {
        #[inject]
        fn new(client: Arc<Client>, url: Url) -> Self {
            Self {
                label: format!("{}:{}", url.0, client.port.0),
                url,
                client,
            }
        }
    }

    injecta::container! {
        struct App {
            instance Url,
            instance Port,
            singleton Arc<Client>,
            transient Service,
        }
    }

    #[test]
    fn derive_fields_and_constructor_arguments_are_injected() {
        let app = App::new(Url("db"), Port(5432));
        let client = app.get::<Arc<Client>>();
        assert_eq!(
            (
                &client.url,
                client.port,
                client.retries,
                client.backoff.as_slice()
            ),
            (&Url("db"), Port(5432), 0, &[1, 2][..])
        );
        let service = app.resolve::<Service>();
        assert_eq!(
            (service.label.as_str(), &service.url),
            ("db:5432", &Url("db"))
        );
        assert!(Arc::ptr_eq(&service.client, client));
    }

    #[test]
    fn build_constructs_unregistered_types_from_registered_entries() {
        let app = App::new(Url("x"), Port(1));
        let client: Client = app.build();
        assert_eq!(client.url, Url("x"));
    }
}

mod storage_types {
    use super::{SingleThread, ThreadSafe};
    use injecta::{SingletonCell, Storage};

    #[test]
    fn storage_cells_cache_the_first_value() {
        let shared = <ThreadSafe as Storage>::Cell::<u8>::empty();
        let local = <SingleThread as Storage>::Cell::<u8>::empty();
        assert_eq!(
            (*shared.get_or_init(|| 1), *shared.get_or_init(|| 2)),
            (1, 1)
        );
        assert_eq!((*local.get_or_init(|| 3), *local.get_or_init(|| 4)), (3, 3));
    }
}
