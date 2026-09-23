#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Phase 1 boundaries between use-case orchestration and external systems.

mod bootstrap;
mod error;
mod metadata;

pub use bootstrap::{
    BootstrapStore, LifecycleLock, LifecycleLockGuard, LifecycleScope, PublishResult,
};
pub use error::{PortConflict, PortError, PortErrorKind};
pub use metadata::MetadataStore;
