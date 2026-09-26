//! P0-07 removal-boundary experiment retained alongside the promoted Git parser.
//!
//! Tracked-status parsing and aggregation now have one production source in
//! `thinws-core`; the historical experiment keeps its public names via re-export.

#![forbid(unsafe_code)]

pub mod removal_log;

pub use thinws_core::{
    DiscoveryCompleteness, GitState, RepositoryState, StatusField, StatusParseError,
    aggregate_git_state, parse_tracked_change_count,
};

/// Result of validating the controlled workspace path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathValidation {
    Valid,
    Failed,
}

/// Result of validating the expected workspace volume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeValidation {
    Valid,
    Failed,
}

/// Best-effort process occupancy evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessUse {
    NoEvidence,
    ScanIncomplete,
    ConfirmedInUse,
}

/// Whether removal is ordinary or explicitly forced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalMode {
    Normal,
    Force,
}

/// Facts consumed by the pure removal policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemovalPreflight {
    pub path: PathValidation,
    pub volume: VolumeValidation,
    pub process_use: ProcessUse,
    pub git: GitState,
}

/// A non-bypassable or ordinary-removal refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalRefusal {
    UnsafePath,
    VolumeMismatch,
    ConfirmedInUse,
    GitCheckIncomplete,
    TrackedChanges,
}

/// Warning retained when process scanning was incomplete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalWarning {
    ProcessScanIncomplete,
}

/// Pure decision made before any removal side effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalDecision {
    Proceed { warning: Option<RemovalWarning> },
    Refuse(RemovalRefusal),
}

/// Applies removal precedence and the deliberately narrow force boundary.
pub fn decide_removal(preflight: RemovalPreflight, mode: RemovalMode) -> RemovalDecision {
    if preflight.path == PathValidation::Failed {
        return RemovalDecision::Refuse(RemovalRefusal::UnsafePath);
    }
    if preflight.volume == VolumeValidation::Failed {
        return RemovalDecision::Refuse(RemovalRefusal::VolumeMismatch);
    }
    if preflight.process_use == ProcessUse::ConfirmedInUse {
        return RemovalDecision::Refuse(RemovalRefusal::ConfirmedInUse);
    }
    if mode == RemovalMode::Normal {
        match preflight.git {
            GitState::Unknown => {
                return RemovalDecision::Refuse(RemovalRefusal::GitCheckIncomplete);
            }
            GitState::Dirty => return RemovalDecision::Refuse(RemovalRefusal::TrackedChanges),
            GitState::NotApplicable | GitState::Clean => {}
        }
    }

    let warning = (preflight.process_use == ProcessUse::ScanIncomplete)
        .then_some(RemovalWarning::ProcessScanIncomplete);
    RemovalDecision::Proceed { warning }
}
