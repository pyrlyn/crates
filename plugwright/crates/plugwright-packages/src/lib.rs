//! Installed plugin packages on disk, the same for every plugwright backend:
//! where a package lives, how it is digested, staged, activated, rolled back,
//! linked for development and removed, and how user and project packages are
//! discovered over the application's own [`Manifest`].
//!
//! - [`discover`] — user and project packages, generic over [`Manifest`].
//! - [`layout`] — stage a package, swap `current` / `previous`, `link`, and
//!   `remove` inside the plugins root.
//! - [`digest`] — [`package_digest`], the SHA-256 both of those agree on.

pub mod digest;
pub mod discover;
pub mod layout;

pub use digest::package_digest;
pub use discover::{Discovered, MANIFEST_FILE, Manifest, Package, Source, State, discover, load_package};
pub use layout::{CURRENT, LINK, PREVIOUS, activate, link, plugin_dir, read_pointer, remove, short, stage};
