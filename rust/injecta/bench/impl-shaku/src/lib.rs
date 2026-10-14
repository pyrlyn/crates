//! The benchmark graph wired with shaku. Shaku injects only `Interface`
//! trait objects, so every service has a trait and an implementation.

use std::sync::Arc;

use shaku::{Component, HasComponent, HasProvider, Interface, Provider, module};

pub trait AppConfig: Interface {
    fn pool_size(&self) -> u32;
    fn timeout_ms(&self) -> u64;
}

#[derive(Component)]
#[shaku(interface = AppConfig)]
pub struct ConfigImpl {
    pool_size: u32,
    timeout_ms: u64,
}

impl AppConfig for ConfigImpl {
    fn pool_size(&self) -> u32 {
        self.pool_size
    }
    fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
}

pub trait Logger: Interface {
    fn level(&self) -> u8;
}

#[derive(Component)]
#[shaku(interface = Logger)]
pub struct ConsoleLogger {
    #[shaku(default = 2)]
    level: u8,
}

impl Logger for ConsoleLogger {
    fn level(&self) -> u8 {
        self.level
    }
}

pub trait Database: Interface {
    fn pool_size(&self) -> u32;
}

#[derive(Component)]
#[shaku(interface = Database)]
pub struct DatabaseImpl {
    #[shaku(inject)]
    config: Arc<dyn AppConfig>,
    #[shaku(inject)]
    #[allow(dead_code)]
    logger: Arc<dyn Logger>,
}

impl Database for DatabaseImpl {
    fn pool_size(&self) -> u32 {
        self.config.pool_size()
    }
}

pub trait Cache: Interface {
    fn capacity(&self) -> u32;
}

#[derive(Component)]
#[shaku(interface = Cache)]
pub struct CacheImpl {
    #[shaku(inject)]
    config: Arc<dyn AppConfig>,
}

impl Cache for CacheImpl {
    fn capacity(&self) -> u32 {
        self.config.pool_size() * 16
    }
}

pub trait UserRepo {
    fn db(&self) -> &Arc<dyn Database>;
    fn logger(&self) -> &Arc<dyn Logger>;
}

#[derive(Provider)]
#[shaku(interface = UserRepo)]
pub struct UserRepoImpl {
    #[shaku(inject)]
    db: Arc<dyn Database>,
    #[shaku(inject)]
    logger: Arc<dyn Logger>,
}

impl UserRepo for UserRepoImpl {
    fn db(&self) -> &Arc<dyn Database> {
        &self.db
    }
    fn logger(&self) -> &Arc<dyn Logger> {
        &self.logger
    }
}

pub trait UserService {
    fn repo(&self) -> &dyn UserRepo;
    fn cache(&self) -> &Arc<dyn Cache>;
    fn logger(&self) -> &Arc<dyn Logger>;
}

#[derive(Provider)]
#[shaku(interface = UserService)]
pub struct UserServiceImpl {
    #[shaku(provide)]
    repo: Box<dyn UserRepo>,
    #[shaku(inject)]
    cache: Arc<dyn Cache>,
    #[shaku(inject)]
    logger: Arc<dyn Logger>,
}

impl UserService for UserServiceImpl {
    fn repo(&self) -> &dyn UserRepo {
        &*self.repo
    }
    fn cache(&self) -> &Arc<dyn Cache> {
        &self.cache
    }
    fn logger(&self) -> &Arc<dyn Logger> {
        &self.logger
    }
}

pub trait Handler {
    fn checksum(&self) -> u64;
}

#[derive(Provider)]
#[shaku(interface = Handler)]
pub struct HandlerImpl {
    #[shaku(provide)]
    service: Box<dyn UserService>,
    #[shaku(inject)]
    config: Arc<dyn AppConfig>,
}

impl Handler for HandlerImpl {
    fn checksum(&self) -> u64 {
        u64::from(self.config.pool_size())
            + self.config.timeout_ms()
            + u64::from(self.service.cache().capacity())
            + u64::from(self.service.repo().db().pool_size())
            + u64::from(self.service.logger().level())
            + u64::from(self.service.repo().logger().level())
    }
}

// --- wiring ---
module! {
    pub App {
        components = [ConfigImpl, ConsoleLogger, DatabaseImpl, CacheImpl],
        providers = [UserRepoImpl, UserServiceImpl, HandlerImpl]
    }
}

pub fn new_app() -> App {
    App::builder()
        .with_component_parameters::<ConfigImpl>(ConfigImplParameters {
            pool_size: 8,
            timeout_ms: 500,
        })
        .build()
}

pub fn resolve_handler(app: &App) -> u64 {
    let handler: Box<dyn Handler> = app.provide().unwrap();
    handler.checksum()
}

pub fn resolve_singleton(app: &App) -> u64 {
    let db: Arc<dyn Database> = app.resolve();
    u64::from(db.pool_size())
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
