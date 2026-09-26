#![forbid(unsafe_code)]

//! System-Git adapter for bounded, read-only workspace inspection.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use thinws_core::AbsolutePath;
use thinws_ports::{GitInspection, GitInspector};

mod git_query;

/// Exercises only in-memory Git parsers in fuzz builds.
#[cfg(fuzzing)]
pub use git_query::exercise_pure_parsers;

/// Fixed system-Git implementation of the read-only inspection Port.
pub struct SystemGitInspector;

impl GitInspector for SystemGitInspector {
    fn inspect(&self, copy_root: &AbsolutePath) -> GitInspection {
        let root = Path::new(OsStr::from_bytes(copy_root.as_bytes()));
        git_query::inspect(root)
    }
}
