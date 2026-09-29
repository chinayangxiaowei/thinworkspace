#![deny(missing_docs)]
#![deny(unsafe_code)]

//! Non-product Linux reflink experiment for the future file materializer.

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod ffi;

#[cfg(target_os = "linux")]
pub use ffi::clone_file_data;
