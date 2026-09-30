//! Synchronous cleanup JSONL in the verified Linux control root.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Component;

use rustix::fs::{Mode, OFlags};
use serde_json::{Value, json};
use thinws_core::{AbsolutePath, GitState, ProcessUse, RemovalMode, RemovalRefusal};
use thinws_ports::{PortError, PortErrorKind, RemovalLogEvent, RemovalLogRecord, WorkspaceRemoval};

use crate::lock::{PrivateDirectory, revalidate_private_directory};
use crate::publication::{sync_directory, validate_private_file};

const LOG_NAME: &str = "operations.jsonl";
const APPEND_FLAGS: OFlags = OFlags::RDWR
    .union(OFlags::APPEND)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);

pub(crate) fn append_removal_record(
    logs: &PrivateDirectory,
    record: &RemovalLogRecord<'_>,
) -> Result<AbsolutePath, PortError> {
    let valid_event = match record.event {
        RemovalLogEvent::Refused => {
            record.protection.is_some() && record.error_code.is_some() && record.outcome.is_none()
        }
        RemovalLogEvent::Started => {
            record.protection.is_none() && record.error_code.is_none() && record.outcome.is_none()
        }
        RemovalLogEvent::Completed => {
            record.protection.is_none() && record.error_code.is_none() && record.outcome.is_some()
        }
        RemovalLogEvent::Failed => record.error_code.is_some() && record.outcome.is_none(),
    };
    if !valid_event || (record.git_check_complete && record.git_state == GitState::Unknown) {
        return Err(PortError::new(
            PortErrorKind::InvalidData,
            "cleanup log event fields are inconsistent",
        ));
    }
    let repositories = record
        .repositories
        .iter()
        .map(|repository| {
            let path = repository.relative_path();
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
            {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "cleanup log repository path is not relative",
                ));
            }
            let bytes = path.as_os_str().as_bytes();
            let display = if bytes.is_empty() {
                Some(".")
            } else {
                std::str::from_utf8(bytes).ok()
            };
            Ok(json!({
                "relative_path": display,
                "relative_path_hex": hex_bytes(bytes),
                "tracked_changes": repository.state().tracked_change_count(),
            }))
        })
        .collect::<Result<Vec<Value>, PortError>>()?;
    let (result, removed_root_entries) = match record.outcome {
        Some(WorkspaceRemoval::AlreadyAbsent) => (Some("already-absent"), None),
        Some(WorkspaceRemoval::Removed { root_entries }) => (Some("removed"), Some(root_entries)),
        None => (None, None),
    };
    let value = json!({
        "timestamp_utc_unix_ms": record.occurred_at.get(),
        "operation_id": record.operation_id.to_string(),
        "workspace_id": record.workspace_id.to_string(),
        "event": event_name(record.event),
        "force": record.mode == RemovalMode::Force,
        "git_state": git_state_name(record.git_state),
        "git_check_complete": record.git_check_complete,
        "repositories": repositories,
        "process_use": record.process_use.map(process_use_name),
        "protection_reason": record.protection.map(refusal_name),
        "error_code": record.error_code.map(|code| code.as_str()),
        "result": result,
        "removed_root_entries": removed_root_entries,
    });
    let mut bytes = serde_json::to_vec(&value).map_err(|error| {
        PortError::new(PortErrorKind::InvalidData, "serialize cleanup log event").with_source(error)
    })?;
    bytes.push(b'\n');

    revalidate_private_directory(logs)?;
    let (mut file, identity) = open_or_create_log(logs)?;
    validate_private_file(logs, &file, LOG_NAME, identity)?;
    let length = file
        .metadata()
        .map_err(|error| io_error("inspect cleanup log length", error))?
        .len();
    if length != 0 {
        file.seek(SeekFrom::End(-1))
            .map_err(|error| io_error("inspect cleanup log tail", error))?;
        let mut tail = [0_u8; 1];
        file.read_exact(&mut tail)
            .map_err(|error| io_error("read cleanup log tail", error))?;
        if tail[0] != b'\n' {
            bytes.insert(0, b'\n');
        }
    }
    file.write_all(&bytes)
        .map_err(|error| io_error("append cleanup log event", error))?;
    file.sync_all()
        .map_err(|error| io_error("sync cleanup log event", error))?;
    validate_private_file(logs, &file, LOG_NAME, identity)?;
    sync_directory(&logs.fd)?;
    revalidate_private_directory(logs)?;
    AbsolutePath::try_from_bytes(logs.path().join(LOG_NAME).as_os_str().as_bytes().to_vec())
        .map_err(|error| {
            PortError::new(PortErrorKind::InvalidLayout, "derive cleanup log path")
                .with_source(error)
        })
}

fn open_or_create_log(logs: &PrivateDirectory) -> Result<(File, (u64, u64)), PortError> {
    let fd = match rustix::fs::openat(&logs.fd, LOG_NAME, APPEND_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => rustix::fs::openat(
            &logs.fd,
            LOG_NAME,
            APPEND_FLAGS | OFlags::CREATE | OFlags::EXCL,
            Mode::from_bits_retain(0o600),
        )
        .map_err(|error| io_error("create cleanup log file", error))?,
        Err(error) => return Err(io_error("open cleanup log file", error)),
    };
    let file = File::from(fd);
    let stat = rustix::fs::fstat(file.as_fd())
        .map_err(|error| io_error("inspect cleanup log file", error))?;
    Ok((file, (stat.st_dev, stat.st_ino)))
}

fn event_name(event: RemovalLogEvent) -> &'static str {
    match event {
        RemovalLogEvent::Refused => "refused",
        RemovalLogEvent::Started => "started",
        RemovalLogEvent::Completed => "completed",
        RemovalLogEvent::Failed => "failed",
    }
}

fn git_state_name(state: GitState) -> &'static str {
    match state {
        GitState::NotApplicable => "not-applicable",
        GitState::Clean => "clean",
        GitState::Dirty => "dirty",
        GitState::Unknown => "unknown",
    }
}

fn process_use_name(state: ProcessUse) -> &'static str {
    match state {
        ProcessUse::NoEvidence => "no-evidence",
        ProcessUse::ScanIncomplete => "scan-incomplete",
        ProcessUse::ConfirmedInUse => "confirmed-in-use",
    }
}

fn refusal_name(refusal: RemovalRefusal) -> &'static str {
    match refusal {
        RemovalRefusal::ConfirmedInUse => "confirmed-in-use",
        RemovalRefusal::GitCheckIncomplete => "git-check-incomplete",
        RemovalRefusal::TrackedChanges => "tracked-changes",
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[usize::from(byte >> 4)] as char);
        result.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    result
}

fn io_error(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(error)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use thinws_core::{ErrorCode, OperationId, RepositoryState, UnixMillis, WorkspaceId};
    use thinws_ports::RepositoryInspection;

    use super::*;
    use crate::lock::prepare_private_directory;

    fn fixture() -> (tempfile::TempDir, PrivateDirectory) {
        let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-linux-operation-log-")
            .tempdir_in(root)
            .unwrap();
        let logs = prepare_private_directory(&fixture.path().join("logs")).unwrap();
        (fixture, logs)
    }

    fn record() -> RemovalLogRecord<'static> {
        RemovalLogRecord {
            occurred_at: UnixMillis::new(1_700_000_000_123).unwrap(),
            operation_id: "op_01890a5d-ac96-774b-bd5b-55c7b8d09f51"
                .parse::<OperationId>()
                .unwrap(),
            workspace_id: "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
                .parse::<WorkspaceId>()
                .unwrap(),
            event: RemovalLogEvent::Started,
            mode: RemovalMode::Force,
            git_state: GitState::Unknown,
            git_check_complete: false,
            repositories: &[],
            process_use: None,
            protection: None,
            error_code: None,
            outcome: None,
        }
    }

    fn assert_invalid(logs: &PrivateDirectory, record: &RemovalLogRecord<'_>) {
        assert_eq!(
            append_removal_record(logs, record).unwrap_err().kind(),
            PortErrorKind::InvalidData
        );
        assert!(!logs.path().join(LOG_NAME).exists());
    }

    #[test]
    fn every_event_field_is_checked_independently_before_log_creation() {
        let (_fixture, logs) = fixture();
        let mut record = record();

        record.event = RemovalLogEvent::Refused;
        record.error_code = Some(ErrorCode::WorkspaceBusy);
        assert_invalid(&logs, &record);
        record.error_code = None;
        record.protection = Some(RemovalRefusal::ConfirmedInUse);
        assert_invalid(&logs, &record);
        record.error_code = Some(ErrorCode::WorkspaceBusy);
        record.outcome = Some(WorkspaceRemoval::AlreadyAbsent);
        assert_invalid(&logs, &record);

        record = self::record();
        record.protection = Some(RemovalRefusal::TrackedChanges);
        assert_invalid(&logs, &record);
        record = self::record();
        record.error_code = Some(ErrorCode::WorkspaceDirty);
        assert_invalid(&logs, &record);
        record = self::record();
        record.outcome = Some(WorkspaceRemoval::AlreadyAbsent);
        assert_invalid(&logs, &record);

        record.event = RemovalLogEvent::Completed;
        record.outcome = None;
        assert_invalid(&logs, &record);
        record.outcome = Some(WorkspaceRemoval::AlreadyAbsent);
        record.error_code = Some(ErrorCode::Filesystem);
        assert_invalid(&logs, &record);
        record.error_code = None;
        record.protection = Some(RemovalRefusal::GitCheckIncomplete);
        assert_invalid(&logs, &record);

        record = self::record();
        record.event = RemovalLogEvent::Failed;
        assert_invalid(&logs, &record);
        record.error_code = Some(ErrorCode::Filesystem);
        record.outcome = Some(WorkspaceRemoval::AlreadyAbsent);
        assert_invalid(&logs, &record);
    }

    #[test]
    fn incomplete_git_and_complete_clean_are_both_valid_log_evidence() {
        let (_fixture, logs) = fixture();
        let mut record = record();
        append_removal_record(&logs, &record).unwrap();
        record.git_state = GitState::Clean;
        record.git_check_complete = true;
        append_removal_record(&logs, &record).unwrap();
        let contents = std::fs::read_to_string(logs.path().join(LOG_NAME)).unwrap();
        let lines = contents.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(lines[0]).unwrap()["git_state"],
            "unknown"
        );
        assert_eq!(
            serde_json::from_str::<Value>(lines[1]).unwrap()["git_state"],
            "clean"
        );
    }

    #[test]
    fn relative_parent_repository_is_rejected_before_log_creation() {
        let (_fixture, logs) = fixture();
        let repositories = [RepositoryInspection::new(
            PathBuf::from("nested/../escape"),
            RepositoryState::Clean,
            vec![],
        )];
        let mut record = record();
        record.repositories = &repositories;
        assert_invalid(&logs, &record);
    }

    #[test]
    fn cleanup_log_names_and_byte_encoding_are_exact() {
        assert_eq!(event_name(RemovalLogEvent::Refused), "refused");
        assert_eq!(event_name(RemovalLogEvent::Started), "started");
        assert_eq!(event_name(RemovalLogEvent::Completed), "completed");
        assert_eq!(event_name(RemovalLogEvent::Failed), "failed");
        assert_eq!(git_state_name(GitState::NotApplicable), "not-applicable");
        assert_eq!(git_state_name(GitState::Clean), "clean");
        assert_eq!(git_state_name(GitState::Dirty), "dirty");
        assert_eq!(git_state_name(GitState::Unknown), "unknown");
        assert_eq!(process_use_name(ProcessUse::NoEvidence), "no-evidence");
        assert_eq!(
            process_use_name(ProcessUse::ScanIncomplete),
            "scan-incomplete"
        );
        assert_eq!(
            process_use_name(ProcessUse::ConfirmedInUse),
            "confirmed-in-use"
        );
        assert_eq!(
            refusal_name(RemovalRefusal::ConfirmedInUse),
            "confirmed-in-use"
        );
        assert_eq!(
            refusal_name(RemovalRefusal::GitCheckIncomplete),
            "git-check-incomplete"
        );
        assert_eq!(
            refusal_name(RemovalRefusal::TrackedChanges),
            "tracked-changes"
        );
        assert_eq!(hex_bytes(&[0x00, 0x0f, 0x10, 0xa5, 0xff]), "000f10a5ff");
    }
}
