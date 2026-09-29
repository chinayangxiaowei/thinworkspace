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
