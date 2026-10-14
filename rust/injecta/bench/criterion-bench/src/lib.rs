//! Containers the criterion suite measures beyond the shared comparison
//! graph: the same graph with `SingleThread` storage, a per-request scope,
//! and generated deep chains (`chains`, written by `gen_chains.py`).

use std::sync::Arc;

use impl_injecta::{
    Cache, Config, ConsoleLogger, Database, Handler, Logger, UserRepo, UserService,
};
use injecta::{Injectable, Resolve, SingleThread};

pub mod chains;

/// The bench configuration every container starts from.
#[must_use]
pub fn config() -> Config {
    Config {
        pool_size: 8,
        timeout_ms: 500,
    }
}

injecta::container! {
    /// The comparison graph with the default `ThreadSafe` (`OnceLock`) storage.
    pub struct App {
        instance Config,
        singleton Arc<dyn Logger> = |c| Arc::new(c.build::<ConsoleLogger>()),
        singleton Arc<Database>,
        singleton Arc<Cache>,
        transient UserRepo,
        transient UserService,
        transient Handler,
    }

    /// One request: its id, a per-request session and a handler that uses both.
    pub scope Request {
        instance RequestId,
        scoped Arc<Session>,
        transient RequestHandler,
    }
}

injecta::container! {
    /// The comparison graph with `SingleThread` (`OnceCell`) storage.
    #[injecta(storage = SingleThread)]
    pub struct StApp {
        instance Config,
        singleton Arc<dyn Logger> = |c| Arc::new(c.build::<ConsoleLogger>()),
        singleton Arc<Database>,
        singleton Arc<Cache>,
        transient UserRepo,
        transient UserService,
        transient Handler,
    }
}

/// Request id, an `instance` of the scope.
#[derive(Clone, Copy)]
pub struct RequestId(pub u64);

/// Per-request state, `scoped`.
#[derive(Injectable)]
pub struct Session {
    /// The request it belongs to.
    pub id: RequestId,
    /// Shared root singleton.
    pub db: Arc<Database>,
}

/// Per-resolve handler mixing scope and root values.
#[derive(Injectable)]
pub struct RequestHandler {
    /// Scoped.
    pub session: Arc<Session>,
    /// Root transient, rebuilt in the scope.
    pub repo: UserRepo,
}

impl RequestHandler {
    /// Something to return so the work is not optimized away.
    #[must_use]
    pub fn checksum(&self) -> u64 {
        self.session.id.0
            + u64::from(self.session.db.config.pool_size)
            + u64::from(self.repo.logger.level())
    }
}

/// A test double for the override benchmark.
pub struct FakeLogger;

impl Logger for FakeLogger {
    fn level(&self) -> u8 {
        0
    }
}
