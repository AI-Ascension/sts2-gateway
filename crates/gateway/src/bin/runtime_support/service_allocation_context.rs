// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::RecoveryLease;

use super::RuntimeService;

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
    service: &RuntimeService,
    lease: &RecoveryLease,
) -> Result<Value, &'static str> {
    let Some(fence) = service.recovery_fence.as_ref() else {
        return Err("recovery_host_fence_required");
    };
    if lease.deployment_id != fence.deployment_id
        || lease.instance_id != fence.instance_id
        || lease.instance_incarnation != fence.instance_incarnation
        || lease.boot_id != fence.boot_id
        || lease.authority_generation != fence.authority_generation
    {
        return Err("recovery_allocation_authority_mismatch");
    }

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
        "current_fence": super::recovery_wire::fence_value(fence),
    }))
}
