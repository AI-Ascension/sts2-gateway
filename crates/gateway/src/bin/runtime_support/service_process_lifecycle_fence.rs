// SPDX-License-Identifier: MIT

//! Lease-fence decision for the attached process-lifecycle adapter.
//!
//! ADR 0024 records lease liveness for an attached deployment as the adapter's own
//! `lease_active` gate rather than a wall-clock deadline: the gateway has no durable expiry it
//! could compare against, and inventing one would make a live lease expire on a timer nobody
//! configured.
//!
//! So liveness is enforced where it is actually known — the HTTP gate in `service_lease.rs`, which
//! runs on every request before dispatch and rejects an inactive, revoked, shut-down, or
//! unauthenticated lease. This port therefore decides the *identity* question, which is the part
//! the coordinator cannot answer for itself: it compares all five proof fields exactly, so a
//! caller cannot present a lease, caller, session, epoch, or instance it did not authenticate
//! with.
//!
//! Expiry is deliberately not re-derived here. A clock deadline compared inside this port would
//! be a second, weaker copy of a decision the HTTP gate already makes with more information, and
//! it would reject a request that the gate accepted whenever the two horizons disagreed.
//!
//! Consequence worth stating plainly: the bound lease's `expires_at` is never read on this path,
//! so it carries no authority. `liveness_is_decided_by_the_http_gate_not_the_lifecycle_fence`
//! pins both halves, so a future change that starts trusting the field here, or that drops the
//! gate's check, fails a test rather than quietly creating a lease nothing enforces.

use sts2_gateway::{FenceFailure, InstanceId, Lease, LeaseDecisionPort, LeaseProof, Tick};

/// Enforces exact lease identity for the attached adapter, deferring liveness to the HTTP gate.
pub(super) struct AttachedLeaseFence;

impl LeaseDecisionPort for AttachedLeaseFence {
    fn check_fence(
        &mut self,
        current: Option<Lease>,
        target: InstanceId,
        proof: LeaseProof,
        _now: Tick,
    ) -> Result<(), FenceFailure> {
        if proof.instance_id() != target {
            return Err(FenceFailure::WrongInstance);
        }
        // `authenticate` binds the lease before calling this port, so a missing value means the
        // coordinator was asked to decide without a bound lease: fail closed.
        let Some(current) = current else {
            return Err(FenceFailure::Missing);
        };
        // Each of these is an exact equality on a gateway-configured value. No arm can widen
        // authority, and an unknown caller/session/lease/epoch cannot be expressed by a caller.
        if current.instance_id() != proof.instance_id() {
            return Err(FenceFailure::WrongInstance);
        }
        if current.caller_id() != proof.caller_id() {
            return Err(FenceFailure::WrongCaller);
        }
        if current.session_id() != proof.session_id() {
            return Err(FenceFailure::WrongSession);
        }
        if current.lease_id() != proof.lease_id() {
            return Err(FenceFailure::WrongLease);
        }
        if current.epoch() != proof.epoch() {
            return Err(FenceFailure::StaleEpoch);
        }
        Ok(())
    }
}
