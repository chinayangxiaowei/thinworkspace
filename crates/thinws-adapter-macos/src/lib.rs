#![deny(unsafe_code)]
#![deny(missing_docs)]

//! macOS bootstrap document and lifecycle-lock Adapter.

mod destroy;
mod document;
#[allow(unsafe_code)]
mod ffi;
mod filesystem;
mod lock;
mod materializer;
mod operation_log;
mod probe;
mod process;
mod store;
mod volume;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use document::{
    DocumentError, MAX_DOCUMENT_BYTES, decode_bootstrap_config, decode_root_marker,
};
pub use lock::MacOsLockGuard;
pub use materializer::{ApfsCloneMaterializer, FullCopyMaterializer};
pub use store::{
    MacOsDataRootLayout, MacOsInitializingProof, MacOsPreparedDataRoot, MacOsPreparedWorkspace,
};
use thinws_ports::{PortError, PortErrorKind};

/// macOS implementation shared by BootstrapStore and LifecycleLock.
#[derive(Clone)]
pub struct MacOsHostAdapter {
    bootstrap_dir: PathBuf,
    token: Arc<()>,
}

impl MacOsHostAdapter {
    /// Binds the Adapter to a canonical absolute bootstrap directory.
    ///
    /// Production passes the fixed Application Support path; tests inject a
    /// private controlled directory without hidden environment overrides.
    pub fn new(bootstrap_dir: impl Into<PathBuf>) -> Result<Self, PortError> {
        let bootstrap_dir = bootstrap_dir.into();
        filesystem::absolute_from_path(&bootstrap_dir).map_err(|error| {
            PortError::new(PortErrorKind::InvalidData, "validate bootstrap path").with_source(error)
        })?;
        Ok(Self {
            bootstrap_dir,
            token: Arc::new(()),
        })
    }

    /// Returns the configured bootstrap directory for explicit wiring.
    #[must_use]
    pub fn bootstrap_dir(&self) -> &Path {
        &self.bootstrap_dir
    }
}
