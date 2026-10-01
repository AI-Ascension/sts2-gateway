// SPDX-License-Identifier: MIT

//! Direct coverage for the individual clauses of `evaluate_fence`.
//!
//! `evaluate_fence` is the pure authorization predicate behind every gateway lease check. Its
//! existing coverage reaches `StaleEpoch`, `WrongInstance` and `Missing` through gateway flows, but
//! the identity and expiry arms are never asserted directly. Each arm below is therefore unpinned:
//! deleting it leaves every suite green while the fence accepts a lease it must refuse.
//!
//! Leases are obtained from a real `Gateway` allocation rather than constructed, so the identity
//! fields under test are the ones the control plane actually issues.

// The shared control-plane doubles are broader than this file needs.
#[allow(dead_code)]
mod support;

use sts2_gateway::{CallerId, FenceFailure, LeaseProof, SessionId, Tick, evaluate_fence};
use support::{new_gateway, owner, ready_gateway, session};

/// A lease and its instance, taken from a real allocation.
fn admitted() -> Result<(sts2_gateway::InstanceId, sts2_gateway::Lease), String> {
    let (mut gateway, _clock, _process, _readiness, _transport) = new_gateway();
    let allocation = ready_gateway(&mut gateway)?;
    Ok((allocation.instance_id(), allocation.lease()))
}

#[test]
fn an_exactly_matching_live_lease_is_accepted() -> Result<(), String> {
    // The positive control. Without it, a fence that refused everything would satisfy all four
    // negatives below and this file would prove nothing.
    let (instance_id, lease) = admitted()?;
    let now = Tick::from_millis(0);
    assert_eq!(
        evaluate_fence(Some(&lease), instance_id, lease.proof(), now),
        Ok(())
    );
    Ok(())
}

#[test]
fn an_expired_lease_is_refused() -> Result<(), String> {
    // The expiry arm. An expired lease must never authorise a forward; `expires_at <= now` is
    // inclusive, so a lease expiring exactly at `now` is already expired.
    let (instance_id, lease) = admitted()?;
    let proof = lease.proof();
    let at_expiry = Tick::from_millis(lease.expires_at().as_millis());
    assert_eq!(
        evaluate_fence(Some(&lease), instance_id, proof, at_expiry),
        Err(FenceFailure::Expired),
        "a lease must be refused at the exact tick it expires"
    );
    assert_eq!(
        evaluate_fence(
            Some(&lease),
            instance_id,
            proof,
            Tick::from_millis(lease.expires_at().as_millis() + 1),
        ),
        Err(FenceFailure::Expired),
        "a lease must be refused after it expires"
    );
    Ok(())
}

#[test]
fn a_lease_is_refused_for_a_foreign_caller() -> Result<(), String> {
    // The caller arm. A different caller holding an otherwise valid lease must not inherit it.
    let (instance_id, lease) = admitted()?;
    let foreign = LeaseProof::new(
        instance_id,
        CallerId::new(owner().value().wrapping_add(1)),
        session(),
        lease.lease_id(),
        lease.epoch(),
    );
    assert_eq!(
        evaluate_fence(Some(&lease), instance_id, foreign, Tick::from_millis(0)),
        Err(FenceFailure::WrongCaller)
    );
    Ok(())
}

#[test]
fn a_lease_is_refused_for_a_foreign_session() -> Result<(), String> {
    // The session arm. A caller may hold more than one session; a proof minted for another
    // session must not satisfy this lease.
    let (instance_id, lease) = admitted()?;
    let foreign = LeaseProof::new(
        instance_id,
        owner(),
        SessionId::new(session().value().wrapping_add(1)),
        lease.lease_id(),
        lease.epoch(),
    );
    assert_eq!(
        evaluate_fence(Some(&lease), instance_id, foreign, Tick::from_millis(0)),
        Err(FenceFailure::WrongSession)
    );
    Ok(())
}

#[test]
fn a_lease_is_refused_for_a_foreign_lease_id() -> Result<(), String> {
    // The lease-id arm. After a release and re-allocation the instance and epoch may be reused
    // while the lease id differs, so this arm is what stops a replayed proof from a prior lease.
    let (instance_id, lease) = admitted()?;
    let foreign = LeaseProof::new(
        instance_id,
        owner(),
        session(),
        sts2_gateway::LeaseId::new(lease.lease_id().value().wrapping_add(1)),
        lease.epoch(),
    );
    assert_eq!(
        evaluate_fence(Some(&lease), instance_id, foreign, Tick::from_millis(0)),
        Err(FenceFailure::WrongLease)
    );
    Ok(())
}
