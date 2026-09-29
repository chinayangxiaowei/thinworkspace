#![deny(unsafe_code)]
#![deny(missing_docs)]

//! Linux implementations of the existing Phase 1 platform ports.

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod ffi;
#[cfg(target_os = "linux")]
mod mountinfo;
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
