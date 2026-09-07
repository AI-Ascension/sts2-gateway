// SPDX-License-Identifier: MIT

use sts2_gateway::{RecoveryHostLeaseState, RecoveryLease};

use super::{RuntimeService, host_lease::HostLeaseFailure};

#[cfg(test)]
thread_local! {
    static EXPIRE_AFTER_INSTALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(super) fn inject_expiry_after_install() {
    EXPIRE_AFTER_INSTALL.with(|fault| fault.set(true));
}

#[cfg(test)]
pub(super) fn apply_expiry_after_install(service: &mut RuntimeService) {
    if EXPIRE_AFTER_INSTALL.with(|fault| fault.replace(false)) {
        service.recovery_lease_deadline = Some(std::time::Instant::now());
    }
}

#[cfg(test)]
pub(super) fn expiry_fault_pending() -> bool {
    EXPIRE_AFTER_INSTALL.with(std::cell::Cell::get)
}

/// Keep failed-allocation cleanup distinct from intentional revocation. This marker is bound
/// to one in-memory lease and is never restored across gateway boot authority replacement.
pub(super) fn cleanup_unreturned_allocation(service: &mut RuntimeService) {
    let lease = service.recovery_lease.clone();
    if !service.lease_revoked && !service.shutdown_requested {
        service.allocation_cleanup_lease_id = lease.as_ref().map(|lease| lease.lease_id.clone());
    }
    // Close every local admission path before the first fallible durable write.
    service.lease_active = false;
    service.lease_revoked = true;
    service.recovery_lease_deadline = None;
    if let Some(lease) = lease {
        // A failed attempt retains its exact identity and cleanup provenance. Only a durable
        // matching host revoke ACK can release this gate, including on a later retry.
        let _ = service.revoke_host_lease(
            &lease.proof(),
            "shutdown",
            &uuid::Uuid::new_v4().to_string(),
        );
    }
}

pub(super) fn failed_install(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
    failure: HostLeaseFailure,
) {
    // A live pending install can still be retried idempotently with the same grant. An expired
    // grant, or an install already committed before local activation failed, must be retired.
    let installed = service
        .recovery
        .as_ref()
        .and_then(|store| store.host_lease_binding(&lease.lease_id).ok().flatten())
        .is_some_and(|binding| binding.state == RecoveryHostLeaseState::Installed);
    if failure.status == 410 || installed {
        cleanup_unreturned_allocation(service);
    }
}
