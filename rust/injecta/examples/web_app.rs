//! A request-handling app: one root container (config, logger, database
//! pool) and a per-request scope (user id, session, handlers). Run with
//! `cargo run --example web_app`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use injecta::{Container, Injectable, Resolve};

#[derive(Clone, Debug)]
struct Config {
    database_url: String,
}

trait Logger: Send + Sync {
    fn log(&self, line: &str);
}

#[derive(Injectable)]
struct StdoutLogger;

impl Logger for StdoutLogger {
    fn log(&self, line: &str) {
        println!("[log] {line}");
    }
}

/// Stands in for a connection pool; counts the queries it served.
struct Database {
    url: String,
    queries: AtomicU64,
    logger: Arc<dyn Logger>,
}

#[injecta::injectable]
impl Database {
    #[inject]
    fn connect(config: Config, logger: Arc<dyn Logger>) -> Self {
        logger.log(&format!("connecting to {}", config.database_url));
        Self {
            url: config.database_url,
            queries: AtomicU64::new(0),
            logger,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct UserId(u64);

#[derive(Injectable)]
struct Session {
    user: UserId,
    db: Arc<Database>,
}

#[derive(Injectable)]
struct ProfileHandler {
    session: Arc<Session>,
}

impl ProfileHandler {
    fn handle(&self) -> String {
        let n = self.session.db.queries.fetch_add(1, Ordering::Relaxed) + 1;
        let db = &self.session.db;
        db.logger.log(&format!("query #{n} on {}", db.url));
        format!("profile of user {}", self.session.user.0)
    }
}

injecta::container! {
    /// Lives as long as the process.
    pub struct App {
        instance Config,
        singleton Arc<dyn Logger> = |c| Arc::new(c.build::<StdoutLogger>()),
        singleton Arc<Database>,
    }
    /// One per incoming request.
    pub scope Request {
        instance UserId,
        scoped Arc<Session>,
        transient ProfileHandler,
    }
}

fn main() {
    let app = App::new(Config {
        database_url: "postgres://localhost/app".into(),
    });
    print!("{}", App::describe());
    for user in [1, 2] {
        let request = Request::new(&app, UserId(user));
        println!("{}", request.resolve::<ProfileHandler>().handle());
    }
}
