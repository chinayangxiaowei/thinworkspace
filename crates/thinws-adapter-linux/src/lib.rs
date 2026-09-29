#![deny(unsafe_code)]
#![deny(missing_docs)]

//! Linux implementations of the existing Phase 1 platform ports.

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod ffi;
#[cfg(target_os = "linux")]
mod probe;

#[cfg(target_os = "linux")]
pub use probe::LinuxPlatformProbe;
