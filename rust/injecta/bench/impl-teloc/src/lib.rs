//! The benchmark graph wired with teloc. teloc clones only types marked
//! `DependencyClone`, and its `Arc<D>` impl requires `D: Sized`, so the
//! trait-object logger travels in a `SharedLogger` newtype.

use std::sync::Arc;

use teloc::dev::DependencyClone;
use teloc::{Dependency, Resolver, ServiceProvider, inject};

#[derive(Clone)]
pub struct Config {
    pub pool_size: u32,
    pub timeout_ms: u64,
}

impl DependencyClone for Config {}

pub trait Logger: Send + Sync {
    fn level(&self) -> u8;
}

pub struct ConsoleLogger {
    level: u8,
}

impl Logger for ConsoleLogger {
    fn level(&self) -> u8 {
        self.level
    }
}

#[derive(Clone)]
pub struct SharedLogger(pub Arc<dyn Logger>);

impl DependencyClone for SharedLogger {}

pub struct Database {
    pub config: Config,
    pub logger: SharedLogger,
}

#[inject]
impl Database {
    pub fn new(config: Config, logger: SharedLogger) -> Self {
        Self { config, logger }
    }
}

pub struct Cache {
    pub capacity: u32,
}

#[inject]
impl Cache {
    pub fn new(config: Config) -> Self {
        Self {
            capacity: config.pool_size * 16,
        }
    }
}

#[derive(Dependency)]
pub struct UserRepo {
    pub db: Arc<Database>,
    pub logger: SharedLogger,
}

#[derive(Dependency)]
pub struct UserService {
    pub repo: UserRepo,
    pub cache: Arc<Cache>,
    pub logger: SharedLogger,
}

#[derive(Dependency)]
pub struct Handler {
    pub service: UserService,
    pub config: Config,
}

impl Handler {
    pub fn checksum(&self) -> u64 {
        u64::from(self.config.pool_size)
            + self.config.timeout_ms
            + u64::from(self.service.cache.capacity)
            + u64::from(self.service.repo.db.config.pool_size)
            + u64::from(self.service.logger.0.level())
            + u64::from(self.service.repo.logger.0.level())
    }
}

// --- wiring ---
// teloc's provider type names a private type, so it cannot be returned from
// a function; a macro builds it where it is used.
macro_rules! app {
    () => {{
        let config = Config {
            pool_size: 8,
            timeout_ms: 500,
        };
        let logger = SharedLogger(Arc::new(ConsoleLogger { level: 2 }));
        ServiceProvider::new()
            .add_instance(config)
            .add_instance(logger)
            .add_singleton::<Arc<Database>>()
            .add_singleton::<Arc<Cache>>()
            .add_transient::<UserRepo>()
            .add_transient::<UserService>()
            .add_transient::<Handler>()
    }};
}

// --- bench api ---
pub fn handler_resolver() -> impl Fn() -> u64 {
    let app = app!();
    move || {
        let handler: Handler = app.resolve();
        handler.checksum()
    }
}

pub fn singleton_resolver() -> impl Fn() -> u64 {
    let app = app!();
    move || {
        let db: Arc<Database> = app.resolve();
        u64::from(db.config.pool_size)
    }
}

pub fn cold() -> u64 {
    let app = app!();
    let handler: Handler = app.resolve();
    handler.checksum()
}
