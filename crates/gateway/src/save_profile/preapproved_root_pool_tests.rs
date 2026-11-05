// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::preapproved_root_pool::{
    ApprovedRoot, CandidateRootPool, ClaimRequest, OwnerExclusion, PoolError,
};
use super::types::UserDataIdentity;
use crate::process_profile::LaunchProfileId;

fn database() -> PathBuf {
    std::env::temp_dir().join(format!("sts2-root-claim-{}.sqlite", uuid::Uuid::new_v4()))
}

fn root(number: u64) -> ApprovedRoot {
    ApprovedRoot {
        key: format!("root-{number}"),
        profile: LaunchProfileId::new(number),
        namespace: number + 100,
        image_digest: format!("{number:064x}"),
    }
}

fn witness(root: &ApprovedRoot) -> OwnerExclusion {
    OwnerExclusion {
        root: root.clone(),
        owner_epoch: 9,
        process_generation: 17,
        no_retained_owner: true,
        no_unsettled_operation: true,
        no_foreign_marker: true,
    }
}

fn request(identity: u64, operation: &str) -> ClaimRequest {
    ClaimRequest {
        identity: UserDataIdentity::new(identity),
        instance: "instance-1".into(),
        operation: operation.into(),
        lease_epoch: 3,
    }
}

#[test]
fn exact_claim_is_durable_and_idempotent_across_coordinator_restart() -> Result<(), PoolError> {
    let path = database();
    let approved = root(1);
    let first = {
        let mut pool = CandidateRootPool::open(&path, vec![approved.clone()])?;
        pool.reserve(request(1, "op-1"), witness(&approved))?
    };
    let mut reopened = CandidateRootPool::open(&path, vec![approved.clone()])?;
    assert_eq!(
        reopened.reserve(request(1, "op-1"), witness(&approved))?,
        first
    );
    assert_eq!(
        reopened
            .reserve(request(1, "op-other"), witness(&approved))
            .err(),
        Some(PoolError::Conflict)
    );
    assert_eq!(
        reopened
            .reserve(request(2, "op-2"), witness(&approved))
            .err(),
        Some(PoolError::Conflict)
    );
    Ok(())
}

#[test]
fn root_exclusion_requires_positive_owner_witness_not_absence() -> Result<(), PoolError> {
    let path = database();
    let approved = root(1);
    let mut pool = CandidateRootPool::open(&path, vec![approved.clone()])?;
    for field in 0..5 {
        let mut proof = witness(&approved);
        match field {
            0 => proof.no_retained_owner = false,
            1 => proof.no_unsettled_operation = false,
            2 => proof.no_foreign_marker = false,
            3 => proof.owner_epoch = 0,
            _ => proof.process_generation = 0,
        }
        assert_eq!(
            pool.reserve(request(1, "op-1"), proof).err(),
            Some(PoolError::UnverifiedOwner)
        );
    }
    let mut mismatched = witness(&approved);
    mismatched.root.namespace += 1;
    assert_eq!(
        pool.reserve(request(1, "op-1"), mismatched).err(),
        Some(PoolError::UnverifiedOwner)
    );
    assert!(pool.reserve(request(1, "op-1"), witness(&approved)).is_ok());
    Ok(())
}

#[test]
fn competing_coordinators_cannot_reassign_one_fixed_native_root() -> Result<(), PoolError> {
    let path = database();
    let approved = root(1);
    let mut first = CandidateRootPool::open(&path, vec![approved.clone()])?;
    let mut second = CandidateRootPool::open(&path, vec![approved.clone()])?;
    first.reserve(request(1, "op-1"), witness(&approved))?;
    assert_eq!(
        second.reserve(request(2, "op-2"), witness(&approved)).err(),
        Some(PoolError::Conflict)
    );
    Ok(())
}

#[test]
fn orphaned_claim_refuses_catalog_drift_instead_of_reallocating() -> Result<(), PoolError> {
    let path = database();
    let approved = root(1);
    {
        let mut pool = CandidateRootPool::open(&path, vec![approved.clone()])?;
        pool.reserve(request(1, "op-1"), witness(&approved))?;
    }
    let mut changed = approved.clone();
    changed.image_digest = "f".repeat(64);
    assert!(matches!(
        CandidateRootPool::open(&path, vec![changed]),
        Err(PoolError::InvalidCatalog)
    ));
    assert!(matches!(
        CandidateRootPool::open(&path, vec![root(2)]),
        Err(PoolError::InvalidCatalog)
    ));
    Ok(())
}

#[test]
fn invalid_catalog_and_request_are_refused_before_any_claim() -> Result<(), PoolError> {
    let path = database();
    assert!(matches!(
        CandidateRootPool::open(&path, vec![root(1), root(1)]),
        Err(PoolError::InvalidCatalog)
    ));
    let mut invalid = root(1);
    invalid.key = "../foreign".into();
    assert!(matches!(
        CandidateRootPool::open(&path, vec![invalid]),
        Err(PoolError::InvalidCatalog)
    ));
    let approved = root(1);
    let mut pool = CandidateRootPool::open(&path, vec![approved.clone()])?;
    assert_eq!(
        pool.reserve(request(0, "op-1"), witness(&approved)).err(),
        Some(PoolError::InvalidCatalog)
    );
    assert_eq!(
        pool.reserve(request(1, "../foreign"), witness(&approved))
            .err(),
        Some(PoolError::InvalidCatalog)
    );
    Ok(())
}
