#![deny(unsafe_code)]
#![deny(missing_docs)]

//! Linux implementations of the existing Phase 1 platform ports.

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod ffi;
#[cfg(target_os = "linux")]
mod mountinfo;
#[cfg(all(target_os = "linux", fuzzing))]
pub use mountinfo::fuzz_mountinfo;
#[cfg(target_os = "linux")]
mod probe;

#[cfg(target_os = "linux")]
pub use probe::LinuxPlatformProbe;

#[cfg(target_os = "linux")]
mod lock;

#[cfg(target_os = "linux")]
mod control;

#[cfg(target_os = "linux")]
mod document;

/// Exercises the Linux ownership document parser and canonical round-trip in memory.
#[cfg(all(target_os = "linux", fuzzing))]
pub fn fuzz_workspace_ownership_document(bytes: &[u8]) {
    if let Ok(ownership) = document::decode_workspace_ownership(bytes) {
        let encoded = document::encode_workspace_ownership(&ownership)
            .expect("valid ownership has a bounded canonical encoding");
        assert_eq!(
            document::decode_workspace_ownership(&encoded),
            Ok(ownership)
        );
    }
}

#[cfg(target_os = "linux")]
mod layout;

#[cfg(target_os = "linux")]
mod publication;

#[cfg(target_os = "linux")]
mod process;

#[cfg(target_os = "linux")]
mod tree;

#[cfg(target_os = "linux")]
mod materializer;

#[cfg(target_os = "linux")]
mod workspace;

#[cfg(target_os = "linux")]
mod space;

#[cfg(target_os = "linux")]
mod operation_log;

#[cfg(target_os = "linux")]
mod destroy;

#[cfg(target_os = "linux")]
mod store;

#[cfg(target_os = "linux")]
pub use lock::{LinuxHostAdapter, LinuxLockGuard};

#[cfg(target_os = "linux")]
pub use control::LinuxPreparedDataRoot;

#[cfg(target_os = "linux")]
pub use layout::LinuxDataRootLayout;

#[cfg(target_os = "linux")]
pub use publication::LinuxInitializingProof;

#[cfg(target_os = "linux")]
pub use materializer::BtrfsReflinkMaterializer;

#[cfg(target_os = "linux")]
pub use workspace::LinuxPreparedWorkspace;
