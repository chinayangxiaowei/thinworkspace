use thinws_core::{
    GitState, ProcessUse, RemovalDecision, RemovalMode, RemovalRefusal, RemovalWarning,
    decide_removal,
};

#[test]
fn ordinary_removal_only_rejects_tracked_or_incomplete_git() {
    for git in [GitState::Clean, GitState::NotApplicable] {
        assert_eq!(
            decide_removal(git, ProcessUse::NoEvidence, RemovalMode::Normal),
            RemovalDecision::Proceed { warning: None }
        );
    }
    assert_eq!(
        decide_removal(GitState::Dirty, ProcessUse::NoEvidence, RemovalMode::Normal),
        RemovalDecision::Refuse(RemovalRefusal::TrackedChanges)
    );
    assert_eq!(
        decide_removal(
            GitState::Unknown,
            ProcessUse::NoEvidence,
            RemovalMode::Normal
        ),
        RemovalDecision::Refuse(RemovalRefusal::GitCheckIncomplete)
    );
}

#[test]
fn explicit_force_only_bypasses_git_and_never_confirmed_process_use() {
    for git in [
        GitState::Clean,
        GitState::NotApplicable,
        GitState::Dirty,
        GitState::Unknown,
    ] {
        assert_eq!(
            decide_removal(git, ProcessUse::ConfirmedInUse, RemovalMode::Force),
            RemovalDecision::Refuse(RemovalRefusal::ConfirmedInUse)
        );
        assert_eq!(
            decide_removal(git, ProcessUse::NoEvidence, RemovalMode::Force),
            RemovalDecision::Proceed { warning: None }
        );
    }
}

#[test]
fn incomplete_process_scan_warns_but_does_not_block_clean_or_forced_removal() {
    assert_eq!(
        decide_removal(
            GitState::Clean,
            ProcessUse::ScanIncomplete,
            RemovalMode::Normal
        ),
        RemovalDecision::Proceed {
            warning: Some(RemovalWarning::ProcessScanIncomplete)
        }
    );
    assert_eq!(
        decide_removal(
            GitState::Unknown,
            ProcessUse::ScanIncomplete,
            RemovalMode::Force
        ),
        RemovalDecision::Proceed {
            warning: Some(RemovalWarning::ProcessScanIncomplete)
        }
    );
}
