// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::{RecoveryFrameError, RecoveryKind};

pub(super) fn validate_payload(
    kind: RecoveryKind,
    value: &Value,
) -> Result<(), RecoveryFrameError> {
    let object = value.as_object().ok_or(RecoveryFrameError::Invalid)?;
    match kind {
        RecoveryKind::Bootstrap => exact_keys(
            object,
            &[
                "deployment_id",
                "instance_id",
                "instance_incarnation",
                "release",
                "lease_policy",
            ],
        )?,
        RecoveryKind::HostFence => exact_keys(object, &["boot"])?,
        RecoveryKind::LeaseAcquire => exact_keys(object, &["boot", "fence"])?,
        RecoveryKind::LeaseRenew => exact_keys(object, &["lease", "renew_sequence"])?,
        RecoveryKind::LeaseRevoke => exact_keys(object, &["lease", "reason"])?,
        RecoveryKind::OperationIntent => exact_keys(object, &["lease", "operation"])?,
        RecoveryKind::OperationDispatch => exact_keys(object, &["lease", "operation"])?,
        RecoveryKind::OperationLookup => exact_keys(object, &["operation", "lookup_scope"])?,
        RecoveryKind::OperationReconcile => {
            exact_keys(object, &["operation", "strategy", "current_fence"])?
        }
    }
    Ok(())
}

pub(super) fn exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), RecoveryFrameError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(RecoveryFrameError::Invalid);
    }
    Ok(())
}

pub(super) fn actor(value: &Value) -> Result<(), RecoveryFrameError> {
    let object = value.as_object().ok_or(RecoveryFrameError::Invalid)?;
    exact_keys(object, &["principal_id", "role"])?;
    if !super::uuid(&object["principal_id"])
        || !matches!(
            object["role"].as_str(),
            Some("gateway" | "watchdog" | "harness" | "host" | "mod" | "operator")
        )
    {
        return Err(RecoveryFrameError::Invalid);
    }
    Ok(())
}

pub(super) fn auth(value: &Value, capability: &str) -> Result<(), RecoveryFrameError> {
    let object = value.as_object().ok_or(RecoveryFrameError::Invalid)?;
    exact_keys(object, &["principal_id", "capability", "proof"])?;
    if !super::uuid(&object["principal_id"])
        || object["capability"].as_str() != Some(capability)
        || (!object["proof"].is_null() && object["proof"].as_str().is_none())
    {
        return Err(RecoveryFrameError::Invalid);
    }
    if object["proof"]
        .as_str()
        .is_some_and(|proof| proof.is_empty() || proof.len() > 512)
    {
        return Err(RecoveryFrameError::Invalid);
    }
    Ok(())
}
