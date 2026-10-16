//! The benchmark graph wired with injecta.

use std::sync::Arc;

use injecta::{Injectable, Resolve};

#[derive(Clone)]
pub struct Config {
    pub pool_size: u32,
    pub timeout_ms: u64,
}

pub trait Logger: Send + Sync {
    fn level(&self) -> u8;
}

#[derive(Injectable)]
pub struct ConsoleLogger {
    #[inject(value = 2)]
    level: u8,
}

impl Logger for ConsoleLogger {
    fn level(&self) -> u8 {
        self.level
    }
}

#[derive(Injectable)]
pub struct Database {
    pub config: Config,
    pub logger: Arc<dyn Logger>,
}

pub struct Cache {
    pub capacity: u32,
}

#[injecta::injectable]
impl Cache {
    #[inject]
    fn new(config: Config) -> Self {
        Self {
            capacity: config.pool_size * 16,
        }
    }
}

#[derive(Injectable)]
pub struct UserRepo {
    pub db: Arc<Database>,
    pub logger: Arc<dyn Logger>,
}

#[derive(Injectable)]
pub struct UserService {
    pub repo: UserRepo,
    pub cache: Arc<Cache>,
    pub logger: Arc<dyn Logger>,
}

#[derive(Injectable)]
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
            + u64::from(self.service.logger.level())
            + u64::from(self.service.repo.logger.level())
    }
}

// --- wiring ---
injecta::container! {
    pub struct App {
        instance Config,
        singleton Arc<dyn Logger> = |c| Arc::new(c.build::<ConsoleLogger>()),
        singleton Arc<Database>,
        singleton Arc<Cache>,
        transient UserRepo,
        transient UserService,
        transient Handler,
    }
}

pub fn new_app() -> App {
    App::new(Config {
        pool_size: 8,
        timeout_ms: 500,
    })
}

pub fn resolve_handler(app: &App) -> u64 {
    app.resolve::<Handler>().checksum()
}

pub fn resolve_singleton(app: &App) -> u64 {
    u64::from(app.resolve::<Arc<Database>>().config.pool_size)
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
