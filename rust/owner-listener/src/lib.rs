//! Owner-only local RPC transport.
//!
//! Unix listeners bind in a `0700` directory and report the peer's uid and
//! pid through tonic's [`Connected`](tonic::transport::server::Connected)
//! trait. Windows listeners create a named pipe with `first_pipe_instance`,
//! a DACL that names only the current user's SID, and the client pid on each
//! connection.
//!
//! [`service`] installs the process as a login service (launchd, a systemd
//! user unit, or a Task Scheduler logon task). Each backend talks to a
//! [`service::ServiceManager`], so the three operating systems are tested
//! with a fake on any host.
//!
//! Same-user malware is out of scope. Any process of the owning user can
//! connect; the directory mode and the DACL stop other users. This crate
//! does not claim a stronger boundary.
//!
//! aulo still has its own copy of this listener. Adopting this crate is that
//! repository's decision.

#![deny(missing_docs)]

mod error;
mod pipe_name;
mod sddl;

#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
mod security;
pub mod service;
#[cfg(unix)]
pub mod unix;

// A spawned child holds a copy of every open fd until it execs, so a listener
// dropped while another test spawns can still accept the stale-socket probe.
// Tests that spawn or that drop a listener and expect it gone take this lock.
#[cfg(all(test, unix))]
pub(crate) static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub use error::Error;
pub use pipe_name::validate_pipe_name;
pub use sddl::{SidError, user_only_dacl};
