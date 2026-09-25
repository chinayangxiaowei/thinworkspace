//! Versioned, lossless serialization of a successful materialization receipt.

use serde_json::{Value, json};
use thinws_core::{
    CowEvidence, CreatedObjectEvidence, FailedMaterializationAttempt, FallbackReason,
    MaterializationFailureKind, MaterializationMode, MaterializationOutcome,
    MaterializationReceipt, MaterializedEntryKind, MaterializerKind, RollbackEvidence,
    RollbackStatus, TreeDigest,
};

pub(super) const RECEIPT_SCHEMA_VERSION: i64 = 1;

pub(super) fn final_receipt_json(receipt: &MaterializationReceipt) -> Value {
    json!({
        "probe_evidence_digest": hex(&receipt.probe_evidence_digest().as_bytes()),
        "requested_mode": mode(receipt.requested_mode()),
        "effective_mode": mode(receipt.effective_mode()),
        "actual_mode": mode(receipt.actual_mode()),
        "actual_adapter": adapter(receipt.actual_adapter()),
        "outcome": outcome(receipt.outcome()),
        "cow_evidence": cow(receipt.cow_evidence()),
        "source_volume_id": receipt.source_volume_id().map(|value| value.to_string()),
        "target_volume_id": receipt.target_volume_id().map(|value| value.to_string()),
        "created": receipt.created().iter().map(created).collect::<Vec<_>>(),
        "unconfirmed_staging": receipt.unconfirmed_staging().map(created),
        "rollback": rollback(receipt.rollback()),
        "elapsed_millis": receipt.elapsed_millis(),
        "logical_bytes": receipt.logical_bytes(),
        "physical_bytes": receipt.physical_bytes(),
        "regular_file_count": receipt.regular_file_count(),
        "clone_calls_succeeded": receipt.clone_calls_succeeded(),
        "source_manifest_digest": receipt.source_manifest_digest().map(digest),
        "target_manifest_digest": receipt.target_manifest_digest().map(digest),
        "failure_kind": receipt.failure_kind().map(failure),
        "fallback_reason": receipt.fallback_reason().map(fallback),
        "failed_attempts": receipt.failed_attempts().iter().map(failed_attempt).collect::<Vec<_>>(),
    })
}

fn created(value: &CreatedObjectEvidence) -> Value {
    let kind = match value.kind() {
        MaterializedEntryKind::Directory => "directory",
        MaterializedEntryKind::RegularFile => "regular_file",
        MaterializedEntryKind::SymbolicLink => "symbolic_link",
    };
    json!({
        "path_hex": hex(value.path().as_bytes()),
        "kind": kind,
        "identity": value.identity().map(|identity| json!({
            "device": identity.device(), "inode": identity.inode()
        })),
    })
}

fn rollback(value: &RollbackEvidence) -> Value {
    let status = match value.status() {
        RollbackStatus::NotNeeded => "not_needed",
        RollbackStatus::ConfirmedBaseline => "confirmed_baseline",
        RollbackStatus::Incomplete => "incomplete",
    };
    json!({
        "status": status,
        "removed_hex": value.removed().iter().map(|path| hex(path.as_bytes())).collect::<Vec<_>>(),
        "quarantined_hex": value.quarantined().iter().map(|path| hex(path.as_bytes())).collect::<Vec<_>>(),
        "unconfirmed_quarantined_hex": value.unconfirmed_quarantined().iter().map(|path| hex(path.as_bytes())).collect::<Vec<_>>(),
        "remaining": value.remaining().iter().map(created).collect::<Vec<_>>(),
    })
}

fn failed_attempt(value: &FailedMaterializationAttempt) -> Value {
    let evidence = value.evidence();
    json!({
        "probe_evidence_digest": hex(&value.probe_evidence_digest().as_bytes()),
        "attempted_mode": mode(value.attempted_mode()),
        "actual_adapter": adapter(value.actual_adapter()),
        "outcome": outcome(value.outcome()),
        "failure_kind": failure(value.failure_kind()),
        "source_volume_id": value.source_volume_id().map(|id| id.to_string()),
        "target_volume_id": value.target_volume_id().map(|id| id.to_string()),
        "created": value.created().iter().map(created).collect::<Vec<_>>(),
        "rollback": rollback(value.rollback()),
        "elapsed_millis": value.elapsed_millis(),
        "logical_bytes": evidence.logical_bytes(),
        "physical_bytes": evidence.physical_bytes(),
        "regular_file_count": evidence.regular_file_count(),
        "clone_calls_succeeded": value.clone_calls_succeeded(),
        "source_manifest_digest": evidence.source_manifest_digest().map(digest),
        "target_manifest_digest": evidence.target_manifest_digest().map(digest),
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

fn digest(value: TreeDigest) -> String {
    hex(&value.as_bytes())
}

fn mode(value: MaterializationMode) -> &'static str {
    match value {
        MaterializationMode::CowClone => "cow_clone",
        MaterializationMode::FullCopy => "full_copy",
    }
}

fn adapter(value: MaterializerKind) -> &'static str {
    match value {
        MaterializerKind::ApfsFileClone => "apfs_file_clone",
        MaterializerKind::FullCopy => "full_copy",
    }
}

fn outcome(value: MaterializationOutcome) -> &'static str {
    match value {
        MaterializationOutcome::Succeeded => "succeeded",
        MaterializationOutcome::Partial => "partial",
        MaterializationOutcome::Failed => "failed",
    }
}

fn cow(value: CowEvidence) -> &'static str {
    match value {
        CowEvidence::Confirmed => "confirmed",
        CowEvidence::NotUsed => "not_used",
        CowEvidence::Unknown => "unknown",
    }
}

fn fallback(value: FallbackReason) -> &'static str {
    match value {
        FallbackReason::CloneUnsupportedAtPreflight => "clone_unsupported_at_preflight",
        FallbackReason::CloneUnavailableAtRuntime => "clone_unavailable_at_runtime",
    }
}

fn failure(value: MaterializationFailureKind) -> &'static str {
    match value {
        MaterializationFailureKind::PlanStale => "plan_stale",
        MaterializationFailureKind::InvalidLayout => "invalid_layout",
        MaterializationFailureKind::UnsupportedSourceEntry => "unsupported_source_entry",
        MaterializationFailureKind::SourceChanged => "source_changed",
        MaterializationFailureKind::TargetChanged => "target_changed",
        MaterializationFailureKind::CowUnavailable => "cow_unavailable",
        MaterializationFailureKind::NoSpace => "no_space",
        MaterializationFailureKind::ManifestMismatch => "manifest_mismatch",
        MaterializationFailureKind::Filesystem => "filesystem",
    }
}
