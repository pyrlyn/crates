//! Building plugins with cargo, the content-addressed artifact cache, git
//! sources, relinking static libraries into shared ones, and build acceleration.

pub mod accel;
pub mod builder;
pub mod cache;
pub mod git;
pub mod package;
pub mod relink;
mod sources;

pub use accel::Accel;
pub use builder::{BuildOptions, BuildRequest, Builder, CrateType, Target};
pub use cache::{Artifact, Cache};
pub use git::{GitRef, GitSource};
pub use package::{Package, find_package};
pub use relink::RelinkRequest;
