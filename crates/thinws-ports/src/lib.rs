#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Phase 1 boundaries between use-case orchestration and external systems.

mod bootstrap;
mod error;
mod metadata;

pub use bootstrap::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLock, LifecycleLockGuard, LifecycleScope,
    PreparedDataRootEvidence, PublishResult,
};
pub use error::{PortConflict, PortError, PortErrorKind};
pub use metadata::{MetadataSnapshot, MetadataStore, MetadataStoreFactory};
