//! Versioned, lossless serialization of a successful materialization receipt.

use serde_json::{Value, json};
use thinws_core::{
    CowEvidence, CreatedObjectEvidence, FailedMaterializationAttempt, FallbackReason,
    MaterializationFailureKind, MaterializationMode, MaterializationOutcome,
    MaterializationReceipt, MaterializedEntryKind, MaterializerKind, RollbackEvidence,
    RollbackStatus, TreeDigest, VolumeId,
};
use thinws_ports::{FinalMaterializationSummary, PortError, PortErrorKind};

pub(super) const RECEIPT_SCHEMA_VERSION: i64 = 1;

pub(super) fn decode_final_summary(
    version: i64,
    document: &str,
    expected_volume: VolumeId,
) -> Result<FinalMaterializationSummary, PortError> {
    if version != RECEIPT_SCHEMA_VERSION {
        return Err(PortError::new(
            PortErrorKind::UnsupportedVersion,
            "read final receipt schema version",
        ));
    }
    let value: Value = serde_json::from_str(document).map_err(|error| {
        PortError::new(PortErrorKind::InvalidData, "parse final receipt JSON").with_source(error)
    })?;
    let requested_mode = decode_mode(field(&value, "requested_mode")?)?;
    let effective_mode = decode_mode(field(&value, "effective_mode")?)?;
    let actual_mode = decode_mode(field(&value, "actual_mode")?)?;
    let adapter = decode_adapter(field(&value, "actual_adapter")?)?;
    let cow = decode_cow(field(&value, "cow_evidence")?)?;
    let fallback_reason = match value.get("fallback_reason") {
        Some(Value::Null) => None,
        Some(Value::String(reason)) => Some(decode_fallback(reason)?),
        _ => return Err(invalid_receipt()),
    };
    let failed_attempt_count = value
        .get("failed_attempts")
        .and_then(Value::as_array)
        .ok_or_else(invalid_receipt)?
        .len();
    let source_digest = field(&value, "source_manifest_digest")?;
    let target_digest = field(&value, "target_manifest_digest")?;
    let volume = expected_volume.to_string();
    let attempt_count_valid = match fallback_reason {
        None | Some(FallbackReason::CloneUnsupportedAtPreflight) => failed_attempt_count == 0,
        Some(FallbackReason::CloneUnavailableAtRuntime) => failed_attempt_count == 1,
    };
    let checks = [
        field(&value, "outcome")? == "succeeded",
        value.get("failure_kind") == Some(&Value::Null),
        value.get("unconfirmed_staging") == Some(&Value::Null),
        value.pointer("/rollback/status").and_then(Value::as_str) == Some("not_needed"),
        source_digest == target_digest,
        source_digest.len() == 64,
        source_digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        field(&value, "source_volume_id")? == volume,
        field(&value, "target_volume_id")? == volume,
        requested_mode == MaterializationMode::CowClone,
        effective_mode == actual_mode,
        attempt_count_valid,
        matches!(
            (actual_mode, adapter, cow, fallback_reason),
            (
                MaterializationMode::CowClone,
                MaterializerKind::ApfsFileClone,
                CowEvidence::Confirmed | CowEvidence::NotUsed,
                None
            ) | (
                MaterializationMode::FullCopy,
                MaterializerKind::FullCopy,
                CowEvidence::NotUsed,
                Some(_)
            )
        ),
    ];
    if checks.contains(&false) {
        return Err(invalid_receipt());
    }
    Ok(FinalMaterializationSummary::new(
        requested_mode,
        effective_mode,
        actual_mode,
        adapter,
        cow,
        fallback_reason,
        failed_attempt_count,
    ))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, PortError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(invalid_receipt)
}

fn invalid_receipt() -> PortError {
    PortError::new(PortErrorKind::InvalidData, "decode final receipt")
}

fn decode_mode(value: &str) -> Result<MaterializationMode, PortError> {
    match value {
        "cow_clone" => Ok(MaterializationMode::CowClone),
        "full_copy" => Ok(MaterializationMode::FullCopy),
        _ => Err(invalid_receipt()),
    }
}

fn decode_adapter(value: &str) -> Result<MaterializerKind, PortError> {
    match value {
        "apfs_file_clone" => Ok(MaterializerKind::ApfsFileClone),
        "full_copy" => Ok(MaterializerKind::FullCopy),
        _ => Err(invalid_receipt()),
    }
}

fn decode_cow(value: &str) -> Result<CowEvidence, PortError> {
    match value {
        "confirmed" => Ok(CowEvidence::Confirmed),
        "not_used" => Ok(CowEvidence::NotUsed),
        _ => Err(invalid_receipt()),
    }
}

fn decode_fallback(value: &str) -> Result<FallbackReason, PortError> {
    match value {
        "clone_unsupported_at_preflight" => Ok(FallbackReason::CloneUnsupportedAtPreflight),
        "clone_unavailable_at_runtime" => Ok(FallbackReason::CloneUnavailableAtRuntime),
        _ => Err(invalid_receipt()),
    }
}

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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use thinws_core::VolumeId;
    use thinws_ports::PortErrorKind;

    use super::{RECEIPT_SCHEMA_VERSION, decode_final_summary};

    #[test]
    fn stored_summary_rejects_malformed_or_inconsistent_final_receipts() {
        let volume = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let valid = serde_json::json!({
            "requested_mode": "cow_clone",
            "effective_mode": "cow_clone",
            "actual_mode": "cow_clone",
            "actual_adapter": "apfs_file_clone",
            "cow_evidence": "confirmed",
            "fallback_reason": null,
            "failed_attempts": [],
            "source_volume_id": volume.to_string(),
            "target_volume_id": volume.to_string(),
            "source_manifest_digest": "03".repeat(32),
            "target_manifest_digest": "03".repeat(32),
            "outcome": "succeeded",
            "failure_kind": null,
            "unconfirmed_staging": null,
            "rollback": {"status": "not_needed"},
        });
        let decode = |value: &serde_json::Value| {
            decode_final_summary(RECEIPT_SCHEMA_VERSION, &value.to_string(), volume)
        };
        assert!(decode(&valid).is_ok());
        assert_eq!(
            decode_final_summary(2, &valid.to_string(), volume)
                .unwrap_err()
                .kind(),
            PortErrorKind::UnsupportedVersion
        );

        for (pointer, replacement) in [
            ("/actual_mode", serde_json::json!("full_copy")),
            ("/outcome", serde_json::json!("failed")),
            (
                "/source_volume_id",
                serde_json::json!("550e8400-e29b-41d4-a716-446655440001"),
            ),
            (
                "/target_manifest_digest",
                serde_json::json!("04".repeat(32)),
            ),
            (
                "/fallback_reason",
                serde_json::json!("clone_unavailable_at_runtime"),
            ),
            ("/rollback/status", serde_json::json!("incomplete")),
        ] {
            let mut changed = valid.clone();
            *changed.pointer_mut(pointer).unwrap() = replacement;
            assert_eq!(
                decode(&changed).unwrap_err().kind(),
                PortErrorKind::InvalidData
            );
        }
        let mut runtime_fallback = valid.clone();
        runtime_fallback["effective_mode"] = serde_json::json!("full_copy");
        runtime_fallback["actual_mode"] = serde_json::json!("full_copy");
        runtime_fallback["actual_adapter"] = serde_json::json!("full_copy");
        runtime_fallback["cow_evidence"] = serde_json::json!("not_used");
        runtime_fallback["fallback_reason"] = serde_json::json!("clone_unavailable_at_runtime");
        runtime_fallback["failed_attempts"] = serde_json::json!([{"outcome": "failed"}]);
        assert_eq!(decode(&runtime_fallback).unwrap().failed_attempt_count(), 1);
        runtime_fallback["failed_attempts"] = serde_json::json!([]);
        assert_eq!(
            decode(&runtime_fallback).unwrap_err().kind(),
            PortErrorKind::InvalidData
        );
        assert_eq!(
            decode(&serde_json::json!({})).unwrap_err().kind(),
            PortErrorKind::InvalidData
        );
    }
}
