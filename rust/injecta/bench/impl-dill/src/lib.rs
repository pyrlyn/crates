//! The benchmark graph wired with dill (runtime registry). dill hands out
//! components as `Arc<T>`, so transient dependencies are `Arc`s here.

use std::sync::Arc;

use dill::{Catalog, Component, component, interface, scope};

#[derive(Clone)]
pub struct Config {
    pub pool_size: u32,
    pub timeout_ms: u64,
}

pub trait Logger: Send + Sync {
    fn level(&self) -> u8;
}

#[component(pub)]
#[interface(dyn Logger)]
#[scope(dill::Singleton)]
pub struct ConsoleLogger {
    level: u8,
}

impl Logger for ConsoleLogger {
    fn level(&self) -> u8 {
        self.level
    }
}

#[component(pub)]
#[scope(dill::Singleton)]
pub struct Database {
    pub config: Config,
    pub logger: Arc<dyn Logger>,
}

pub struct Cache {
    pub capacity: u32,
}

#[component(pub)]
#[scope(dill::Singleton)]
impl Cache {
    pub fn new(config: Config) -> Self {
        Self {
            capacity: config.pool_size * 16,
        }
    }
}

#[component(pub)]
pub struct UserRepo {
    pub db: Arc<Database>,
    pub logger: Arc<dyn Logger>,
}

#[component(pub)]
pub struct UserService {
    pub repo: Arc<UserRepo>,
    pub cache: Arc<Cache>,
    pub logger: Arc<dyn Logger>,
}

#[component(pub)]
pub struct Handler {
    pub service: Arc<UserService>,
    pub config: Config,
}

impl Handler {
    pub fn checksum(&self) -> u64 {
        u64::from(self.config.pool_size)
            + self.config.timeout_ms
            + u64::from(self.service.cache.capacity)
            + u64::from(self.service.repo.db.config.pool_size)
            + u64::from(self.service.logger.level())
            + u64::from(self.service.repo.logger.level())
    }
}

// --- wiring ---
pub type App = Catalog;

pub fn new_app() -> App {
    Catalog::builder()
        .add_value(Config {
            pool_size: 8,
            timeout_ms: 500,
        })
        .add_builder(ConsoleLogger::builder().with_level(2))
        .add::<Database>()
        .add::<Cache>()
        .add::<UserRepo>()
        .add::<UserService>()
        .add::<Handler>()
        .build()
}

pub fn resolve_handler(app: &App) -> u64 {
    app.get_one::<Handler>().unwrap().checksum()
}

pub fn resolve_singleton(app: &App) -> u64 {
    u64::from(app.get_one::<Database>().unwrap().config.pool_size)
}

// --- bench api ---
pub fn handler_resolver() -> impl Fn() -> u64 {
    let app = new_app();
    move || resolve_handler(&app)
}

pub fn singleton_resolver() -> impl Fn() -> u64 {
    let app = new_app();
    move || resolve_singleton(&app)
}

pub fn cold() -> u64 {
    resolve_handler(&new_app())
}
