//! Hand-written wiring: the baseline every DI crate is measured against.

use std::sync::Arc;

#[derive(Clone)]
pub struct Config {
    pub pool_size: u32,
    pub timeout_ms: u64,
}

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

pub struct Database {
    pub config: Config,
    pub logger: Arc<dyn Logger>,
}

pub struct Cache {
    pub capacity: u32,
}

pub struct UserRepo {
    pub db: Arc<Database>,
    pub logger: Arc<dyn Logger>,
}

pub struct UserService {
    pub repo: UserRepo,
    pub cache: Arc<Cache>,
    pub logger: Arc<dyn Logger>,
}

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
pub struct App {
    config: Config,
    logger: Arc<dyn Logger>,
    db: Arc<Database>,
    cache: Arc<Cache>,
}

pub fn new_app() -> App {
    let config = Config {
        pool_size: 8,
        timeout_ms: 500,
    };
    let logger: Arc<dyn Logger> = Arc::new(ConsoleLogger { level: 2 });
    let db = Arc::new(Database {
        config: config.clone(),
        logger: logger.clone(),
    });
    let cache = Arc::new(Cache {
        capacity: config.pool_size * 16,
    });
    App {
        config,
        logger,
        db,
        cache,
    }
}

pub fn resolve_handler(app: &App) -> u64 {
    let repo = UserRepo {
        db: app.db.clone(),
        logger: app.logger.clone(),
    };
    let service = UserService {
        repo,
        cache: app.cache.clone(),
        logger: app.logger.clone(),
    };
    Handler {
        service,
        config: app.config.clone(),
    }
    .checksum()
}

pub fn resolve_singleton(app: &App) -> u64 {
    let db: Arc<Database> = app.db.clone();
    u64::from(db.config.pool_size)
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
