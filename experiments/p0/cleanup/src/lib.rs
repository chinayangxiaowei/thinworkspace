//! Parsing, removal-policy, and bounded internal Git-query experiments for P0-07.
//!
//! Parsing and policy evaluation are pure. The [`git_query`] module is the only
//! process-I/O boundary and permits four fixed, read-only `/usr/bin/git` query
//! shapes. This crate performs no logging or deletion I/O and is not a Phase 1
//! Port or service implementation.

#![forbid(unsafe_code)]

pub mod git_query;

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroUsize;

/// A field in a Git porcelain-v2 record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusField {
    RecordType,
    Header,
    Xy,
    Submodule,
    HeadMode,
    IndexMode,
    WorktreeMode,
    HeadObject,
    IndexObject,
    StageOneMode,
    StageTwoMode,
    StageThreeMode,
    StageOneObject,
    StageTwoObject,
    StageThreeObject,
    Score,
    Path,
}

/// A structural failure while parsing `git status --porcelain=v2 -z`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StatusParseError {
    UnknownRecord {
        offset: usize,
        record_type: u8,
    },
    InvalidField {
        offset: usize,
        record_type: u8,
        field: StatusField,
    },
    TruncatedRecord {
        offset: usize,
        record_type: Option<u8>,
    },
    MissingRenameSource {
        offset: usize,
    },
}

impl fmt::Display for StatusParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid Git porcelain-v2 output: {self:?}")
    }
}

impl std::error::Error for StatusParseError {}

/// Counts unique tracked paths in NUL-terminated porcelain-v2 output.
pub fn parse_tracked_change_count(output: &[u8]) -> Result<usize, StatusParseError> {
    let mut paths = BTreeSet::new();
    let mut offset = 0;

    while offset < output.len() {
        let record_offset = offset;
        let record_end = output[offset..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|relative| offset + relative)
            .ok_or(StatusParseError::TruncatedRecord {
                offset: record_offset,
                record_type: output.get(record_offset).copied(),
            })?;
        let record = &output[record_offset..record_end];
        offset = record_end + 1;

        let record_type = record.first().copied().unwrap_or(0);
        match record_type {
            b'1' => parse_ordinary(record, record_offset, &mut paths)?,
            b'2' => {
                parse_renamed(record, record_offset, &mut paths)?;
                let source_end = output[offset..]
                    .iter()
                    .position(|byte| *byte == 0)
                    .map(|relative| offset + relative)
                    .ok_or(StatusParseError::MissingRenameSource {
                        offset: record_offset,
                    })?;
                if source_end == offset {
                    return Err(StatusParseError::MissingRenameSource {
                        offset: record_offset,
                    });
                }
                offset = source_end + 1;
            }
            b'u' => parse_unmerged(record, record_offset, &mut paths)?,
            b'?' | b'!' => parse_other_item(record, record_offset, record_type)?,
            b'#' => parse_header(record, record_offset)?,
            _ => {
                return Err(StatusParseError::UnknownRecord {
                    offset: record_offset,
                    record_type,
                });
            }
        }
    }

    Ok(paths.len())
}

fn parse_ordinary(
    record: &[u8],
    offset: usize,
    paths: &mut BTreeSet<Vec<u8>>,
) -> Result<(), StatusParseError> {
    let fields = split_fields::<9>(record, offset, b'1')?;
    validate_xy(fields[1], false, offset, b'1')?;
    validate_submodule(fields[2], offset, b'1')?;
    validate_mode(fields[3], StatusField::HeadMode, offset, b'1')?;
    validate_mode(fields[4], StatusField::IndexMode, offset, b'1')?;
    validate_mode(fields[5], StatusField::WorktreeMode, offset, b'1')?;
    validate_object(fields[6], StatusField::HeadObject, offset, b'1')?;
    validate_object(fields[7], StatusField::IndexObject, offset, b'1')?;

    if fields[1] == b".." || fields[1].iter().any(|byte| matches!(byte, b'R' | b'C')) {
        return Err(invalid_field(offset, b'1', StatusField::Xy));
    }

    // Git collapses a submodule's untracked-only state into `.M`; the submodule
    // flags plus unchanged gitlink modes and object IDs prove that no tracked
    // superproject change is present. Any deviation remains dirty.
    let only_untracked_submodule = fields[1] == b".M"
        && fields[2] == b"S..U"
        && fields[3] == b"160000"
        && fields[3] == fields[4]
        && fields[4] == fields[5]
        && fields[6] == fields[7];
    if !only_untracked_submodule {
        paths.insert(fields[8].to_vec());
    }

    Ok(())
}

fn parse_renamed(
    record: &[u8],
    offset: usize,
    paths: &mut BTreeSet<Vec<u8>>,
) -> Result<(), StatusParseError> {
    let fields = split_fields::<10>(record, offset, b'2')?;
    validate_xy(fields[1], false, offset, b'2')?;
    validate_submodule(fields[2], offset, b'2')?;
    validate_mode(fields[3], StatusField::HeadMode, offset, b'2')?;
    validate_mode(fields[4], StatusField::IndexMode, offset, b'2')?;
    validate_mode(fields[5], StatusField::WorktreeMode, offset, b'2')?;
    validate_object(fields[6], StatusField::HeadObject, offset, b'2')?;
    validate_object(fields[7], StatusField::IndexObject, offset, b'2')?;
    validate_score(fields[8], fields[1], offset)?;
    paths.insert(fields[9].to_vec());
    Ok(())
}

fn parse_unmerged(
    record: &[u8],
    offset: usize,
    paths: &mut BTreeSet<Vec<u8>>,
) -> Result<(), StatusParseError> {
    let fields = split_fields::<11>(record, offset, b'u')?;
    validate_xy(fields[1], true, offset, b'u')?;
    validate_submodule(fields[2], offset, b'u')?;
    validate_mode(fields[3], StatusField::StageOneMode, offset, b'u')?;
    validate_mode(fields[4], StatusField::StageTwoMode, offset, b'u')?;
    validate_mode(fields[5], StatusField::StageThreeMode, offset, b'u')?;
    validate_mode(fields[6], StatusField::WorktreeMode, offset, b'u')?;
    validate_object(fields[7], StatusField::StageOneObject, offset, b'u')?;
    validate_object(fields[8], StatusField::StageTwoObject, offset, b'u')?;
    validate_object(fields[9], StatusField::StageThreeObject, offset, b'u')?;
    paths.insert(fields[10].to_vec());
    Ok(())
}

fn parse_other_item(record: &[u8], offset: usize, record_type: u8) -> Result<(), StatusParseError> {
    if record.len() < 3 || record[1] != b' ' || record[2..].is_empty() {
        return Err(invalid_field(offset, record_type, StatusField::Path));
    }
    Ok(())
}

fn parse_header(record: &[u8], offset: usize) -> Result<(), StatusParseError> {
    if record.len() < 3
        || record[1] != b' '
        || record[2..].is_empty()
        || record[2] == b' '
        || record[2..].iter().any(|byte| matches!(byte, b'\n' | b'\r'))
    {
        return Err(invalid_field(offset, b'#', StatusField::Header));
    }
    Ok(())
}

fn split_fields<const N: usize>(
    record: &[u8],
    offset: usize,
    record_type: u8,
) -> Result<[&[u8]; N], StatusParseError> {
    let mut fields = record.splitn(N, |byte| *byte == b' ');
    let parsed = std::array::from_fn(|_| fields.next().unwrap_or_default());
    if parsed.iter().any(|field| field.is_empty()) {
        return Err(StatusParseError::TruncatedRecord {
            offset,
            record_type: Some(record_type),
        });
    }
    if parsed[0] != [record_type] {
        return Err(invalid_field(offset, record_type, StatusField::RecordType));
    }
    Ok(parsed)
}

fn validate_xy(
    xy: &[u8],
    unmerged: bool,
    offset: usize,
    record_type: u8,
) -> Result<(), StatusParseError> {
    let valid = if unmerged {
        matches!(xy, b"DD" | b"AU" | b"UD" | b"UA" | b"DU" | b"AA" | b"UU")
    } else {
        xy.len() == 2
            && xy
                .iter()
                .all(|byte| matches!(byte, b'.' | b'M' | b'T' | b'A' | b'D' | b'R' | b'C'))
    };
    if valid {
        Ok(())
    } else {
        Err(invalid_field(offset, record_type, StatusField::Xy))
    }
}

fn validate_submodule(
    submodule: &[u8],
    offset: usize,
    record_type: u8,
) -> Result<(), StatusParseError> {
    let valid = submodule == b"N..."
        || (submodule.len() == 4
            && submodule[0] == b'S'
            && matches!(submodule[1], b'.' | b'C')
            && matches!(submodule[2], b'.' | b'M')
            && matches!(submodule[3], b'.' | b'U'));
    if valid {
        Ok(())
    } else {
        Err(invalid_field(offset, record_type, StatusField::Submodule))
    }
}

fn validate_mode(
    mode: &[u8],
    field: StatusField,
    offset: usize,
    record_type: u8,
) -> Result<(), StatusParseError> {
    if mode.len() == 6 && mode.iter().all(|byte| matches!(byte, b'0'..=b'7')) {
        Ok(())
    } else {
        Err(invalid_field(offset, record_type, field))
    }
}

fn validate_object(
    object: &[u8],
    field: StatusField,
    offset: usize,
    record_type: u8,
) -> Result<(), StatusParseError> {
    if matches!(object.len(), 40 | 64) && object.iter().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid_field(offset, record_type, field))
    }
}

fn validate_score(score: &[u8], xy: &[u8], offset: usize) -> Result<(), StatusParseError> {
    let Some(kind) = score.first().copied() else {
        return Err(invalid_field(offset, b'2', StatusField::Score));
    };
    let value = &score[1..];
    let score_matches_xy = xy.contains(&kind);
    let valid = matches!(kind, b'R' | b'C')
        && score_matches_xy
        && !value.is_empty()
        && value.len() <= 3
        && value.iter().all(u8::is_ascii_digit)
        && parse_decimal(value).is_some_and(|value| value <= 100);
    if valid {
        Ok(())
    } else {
        Err(invalid_field(offset, b'2', StatusField::Score))
    }
}

fn parse_decimal(digits: &[u8]) -> Option<u16> {
    digits.iter().try_fold(0_u16, |value, digit| {
        value.checked_mul(10)?.checked_add(u16::from(*digit - b'0'))
    })
}

fn invalid_field(offset: usize, record_type: u8, field: StatusField) -> StatusParseError {
    StatusParseError::InvalidField {
        offset,
        record_type,
        field,
    }
}

/// Completeness of repository discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryCompleteness {
    Complete,
    Incomplete,
}

/// The tracked-content state of one discovered repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryState {
    Clean,
    Dirty { tracked_changes: NonZeroUsize },
    Unknown,
}

impl RepositoryState {
    /// Converts a successful parser count into a repository state.
    pub fn from_tracked_change_count(tracked_changes: usize) -> Self {
        match NonZeroUsize::new(tracked_changes) {
            Some(tracked_changes) => Self::Dirty { tracked_changes },
            None => Self::Clean,
        }
    }

    /// Returns a known count, or `None` when the count is unknown.
    pub fn tracked_change_count(self) -> Option<usize> {
        match self {
            Self::Clean => Some(0),
            Self::Dirty { tracked_changes } => Some(tracked_changes.get()),
            Self::Unknown => None,
        }
    }
}

/// Aggregate Git state used by status and removal policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitState {
    NotApplicable,
    Clean,
    Dirty,
    Unknown,
}

/// Aggregates repository states without allowing incomplete evidence to look clean.
pub fn aggregate_git_state(
    discovery: DiscoveryCompleteness,
    repositories: &[RepositoryState],
) -> GitState {
    if discovery == DiscoveryCompleteness::Incomplete
        || repositories.contains(&RepositoryState::Unknown)
    {
        GitState::Unknown
    } else if repositories
        .iter()
        .any(|state| matches!(state, RepositoryState::Dirty { .. }))
    {
        GitState::Dirty
    } else if repositories.is_empty() {
        GitState::NotApplicable
    } else {
        GitState::Clean
    }
}

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
