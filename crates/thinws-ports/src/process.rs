//! Best-effort observation of external processes using a Workspace copy.

use thinws_core::{AbsolutePath, ProcessUse, UnixMillis};

use crate::PortError;

/// A point-in-time scan result, not a lease on the inspected path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessObservation {
    /// When the scan finished, in UTC Unix milliseconds.
    pub observed_at: UnixMillis,
    /// Confirmed use, no evidence, or incomplete scan.
    pub use_state: ProcessUse,
}

/// Read-only, best-effort inspection of current-user-visible processes.
pub trait ProcessProbe {
    /// Inspects cwd and open-vnode use of an already validated Workspace container.
    ///
    /// The implementation must revalidate process identity beyond a PID before
    /// reporting confirmed use. Inaccessibility is scan-incomplete, never
    /// evidence of no use. This call does not terminate or manage processes.
    fn inspect_workspace(
        &self,
        workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError>;
}
