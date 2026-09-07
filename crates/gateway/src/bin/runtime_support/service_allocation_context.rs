// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::{RecoveryHostLeaseState, RecoveryLease};

use super::{RuntimeService, json_bytes, json_error};

// Use the accepted sideband reason for shutting down an unreturned allocation.
// Correlation is a fresh transport UUID; installation/lease identity stays fixed.
const ALLOCATION_FAILURE_REASON: &str = "shutdown";

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static POST_INSTALL_FENCE_MISMATCH: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(super) fn inject_post_install_fence_mismatch() {
    POST_INSTALL_FENCE_MISMATCH.with(|fault| fault.set(true));
}

#[cfg(test)]
fn apply_post_install_fence_mismatch(service: &mut RuntimeService) {
    if POST_INSTALL_FENCE_MISMATCH.with(|fault| fault.replace(false))
        && let Some(fence) = service.recovery_fence.as_mut()
    {
        fence.boot_id = String::from("00000000-0000-4000-8000-000000000099");
    }
}

/// The digest is the SHA-256 of the owner-local closed allocation-authority
/// schema published under `contract-artifact/runtime-allocation-v1`.
pub(super) const ALLOCATION_SCHEMA_DIGEST: &str =
    "ee967a95e79fb2f157ce58d2b6d857de42b75f1f5ebfeb82dd9672e3b0f7670b";

const ALLOCATION_CONTRACT: &str = "watchdog-runtime-allocation-v1";

/// Build the immutable authority binding that accompanies a recovery lease.
///
/// Every authority field is taken from the durable lease and the current host
/// fence.  The ordinary runtime config is deliberately not consulted: it is
/// only the caller identity used to request allocation and may contain stale
/// lease values after recovery.
pub(super) fn recovery_authority(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
) -> Result<Value, (u16, Vec<u8>)> {
    let fence = validate_current_allocation(service, lease)?;
    Ok(json!({
        "contract": ALLOCATION_CONTRACT,
        "schema_digest": ALLOCATION_SCHEMA_DIGEST,
        "context": {
            "deployment_id": lease.deployment_id,
            "instance_id": lease.instance_id,
            "instance_incarnation": lease.instance_incarnation,
            "boot_id": lease.boot_id,
            "authority_generation": lease.authority_generation,
            "lease_id": lease.lease_id,
            "lease_epoch": lease.lease_epoch,
        },
        "current_fence": super::recovery_wire::fence_value(&fence),
    }))
}
fn validate_current_allocation(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
) -> Result<sts2_gateway::RecoveryHostFence, (u16, Vec<u8>)> {
    let Some(current_lease) = service.recovery_lease.as_ref() else {
        return Err((503, json_error("recovery_lease_required")));
    };
    if current_lease != lease {
        return Err((409, json_error("recovery_allocation_lease_mismatch")));
    }
    if !service.lease_active || service.lease_revoked || service.shutdown_requested {
        return Err((503, json_error("recovery_lease_not_active")));
    }
    if !service
        .recovery_lease_deadline
        .is_some_and(|deadline| std::time::Instant::now() < deadline)
    {
        return Err((410, json_error("recovery_lease_expired")));
    }
    let Some(boot) = service.recovery_boot.clone() else {
        return Err((503, json_error("recovery_boot_required")));
    };
    let Some(fence) = service.recovery_fence.clone() else {
        return Err((503, json_error("recovery_host_fence_required")));
    };
    if lease.deployment_id != boot.deployment_id
        || lease.instance_id != boot.instance_id
        || lease.instance_incarnation != boot.instance_incarnation
        || lease.boot_id != boot.boot_id
        || lease.authority_generation != boot.authority_generation
        || lease.deployment_id != fence.deployment_id
        || lease.instance_id != fence.instance_id
        || lease.instance_incarnation != fence.instance_incarnation
        || lease.boot_id != fence.boot_id
        || lease.authority_generation != fence.authority_generation
    {
        return Err((503, json_error("recovery_allocation_authority_mismatch")));
    }
    validate_durable_allocation(service, lease, &fence)?;
    Ok(fence)
}

fn validate_durable_allocation(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
    fence: &sts2_gateway::RecoveryHostFence,
) -> Result<(), (u16, Vec<u8>)> {
    let now = service.recovery_now_millis();
    let Some(store) = service.recovery.as_mut() else {
        return Err((503, json_error("recovery_persistence_unavailable")));
    };
    let durable_fence = store
        .current_host_fence()
        .map_err(super::recovery_wire::recovery_store_error)?;
    if durable_fence != *fence {
        return Err((503, json_error("recovery_allocation_authority_mismatch")));
    }
    store
        .validate_lease(&lease.proof(), now)
        .map_err(super::recovery_wire::recovery_store_error)?;
    let host_binding = store
        .host_lease_binding(&lease.lease_id)
        .map_err(super::recovery_wire::recovery_store_error)?;
    if !host_binding.is_some_and(|binding| binding.state == RecoveryHostLeaseState::Installed) {
        return Err((503, json_error("recovery_host_lease_required")));
    }
    Ok(())
}

fn allocation_response_failure(
    service: &mut RuntimeService,
    failure: (u16, Vec<u8>),
) -> (u16, Vec<u8>) {
    let already_revoked = service.lease_revoked;
    // Close every local admission path before trying a fallible durable write.
    // A failed write can leave the durable lease ACTIVE; that is not permission
    // to continue after this response has failed.
    service.lease_active = false;
    service.lease_revoked = true;
    service.recovery_lease_deadline = None;
    let Some(lease) = service.recovery_lease.clone() else {
        return failure;
    };
    if service
        .revoke_host_lease(
            &lease.proof(),
            ALLOCATION_FAILURE_REASON,
            &uuid::Uuid::new_v4().to_string(),
        )
        .is_ok()
    {
        // Only acknowledged, durably recorded cleanup permits a fresh epoch.
        // Never undo revocation/stop that preceded this failed response.
        service.lease_revoked = already_revoked || service.shutdown_requested;
    }
    failure
}

pub(super) fn allocation_response(
    service: &mut RuntimeService,
    lease: &sts2_gateway::RecoveryLease,
) -> (u16, Vec<u8>) {
    #[cfg(test)]
    apply_post_install_fence_mismatch(service);
    let recovery_authority = match super::allocation_context::recovery_authority(service, lease) {
        Ok(authority) => authority,
        Err(error) => return allocation_response_failure(service, error),
    };
    (
        200,
        json_bytes(&json!({
            "status": "allocated",
            "instance_id": lease.instance_id,
            "caller_id": service.config.caller_id,
            "session_id": service.config.session_id,
            "lease_id": lease.lease_id,
            "lease_epoch": lease.lease_epoch,
            "fence_token": lease.fence_token,
            "expires_at_millis": lease.expires_at_millis,
            "transport": "attached-loopback",
            "recovery_authority": recovery_authority,
        })),
    )
}
