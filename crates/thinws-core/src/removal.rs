//! Pure cleanup decision after external path and process evidence is collected.

use crate::{GitState, RemovalMode};

/// Best-effort observation of current external process use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessUse {
    /// The scan found no matching cwd/open-vnode evidence.
    NoEvidence,
    /// The scan did not cover every visible process.
    ScanIncomplete,
    /// A revalidated process has a matching cwd/open-vnode.
    ConfirmedInUse,
}

/// Why a cleanup attempt must stop before destructive work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalRefusal {
    /// A revalidated external process occupies the Workspace.
    ConfirmedInUse,
    /// Ordinary cleanup cannot trust an incomplete Git result.
    GitCheckIncomplete,
    /// Ordinary cleanup found currently tracked changes.
    TrackedChanges,
}

/// Non-blocking warning that must remain visible and logged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalWarning {
    /// Process occupancy scanning did not complete.
    ProcessScanIncomplete,
}

/// Pure outcome before any deletion state or filesystem write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalDecision {
    /// The applicable policy permits cleanup, possibly with one warning.
    Proceed {
        /// An incomplete process scan cannot be presented as no use.
        warning: Option<RemovalWarning>,
    },
    /// Cleanup is blocked by one specific protection.
    Refuse(RemovalRefusal),
}

/// Applies only Git and process protections; path, volume, and ownership proof
/// must already have been validated and cannot be bypassed by this function.
#[must_use]
pub fn decide_removal(
    git: GitState,
    process_use: ProcessUse,
    mode: RemovalMode,
) -> RemovalDecision {
    if process_use == ProcessUse::ConfirmedInUse {
        return RemovalDecision::Refuse(RemovalRefusal::ConfirmedInUse);
    }
    if mode == RemovalMode::Normal {
        match git {
            GitState::Unknown => {
                return RemovalDecision::Refuse(RemovalRefusal::GitCheckIncomplete);
            }
            GitState::Dirty => return RemovalDecision::Refuse(RemovalRefusal::TrackedChanges),
            GitState::Clean | GitState::NotApplicable => {}
        }
    }
    RemovalDecision::Proceed {
        warning: (process_use == ProcessUse::ScanIncomplete)
            .then_some(RemovalWarning::ProcessScanIncomplete),
    }
}
