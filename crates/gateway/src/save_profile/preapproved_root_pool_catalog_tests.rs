// SPDX-License-Identifier: MIT

//! Full-pool restart integrity tests for the test-only candidate claim journal.

use std::path::PathBuf;

use super::preapproved_root_pool::{
    ApprovedRoot, CandidateRootPool, ClaimRequest, OwnerExclusion, PoolError,
};
use super::types::UserDataIdentity;
use crate::process_profile::LaunchProfileId;

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "sts2-root-catalog-{}.sqlite",
        uuid::Uuid::new_v4()
    ))
}

fn root(number: u64) -> ApprovedRoot {
    ApprovedRoot {
        key: format!("root-{number}"),
        profile: LaunchProfileId::new(number),
        namespace: number + 100,
        image_digest: format!("{number:064x}"),
    }
}

fn request() -> ClaimRequest {
    ClaimRequest {
        identity: UserDataIdentity::new(7),
        instance: "instance-1".into(),
        operation: "operation-7".into(),
        lease_epoch: 3,
    }
}

fn witness(approved: &ApprovedRoot) -> OwnerExclusion {
    OwnerExclusion {
        root: approved.clone(),
        instance: "instance-1".into(),
        lease_epoch: 3,
        owner_epoch: 9,
        process_generation: 17,
        no_retained_owner: true,
        no_unsettled_operation: true,
        no_foreign_marker: true,
    }
}

#[test]
fn empty_journal_pins_all_roots_even_before_first_claim() -> Result<(), PoolError> {
    let path = database();
    let first = root(1);
    let second = root(2);
    {
        let _pool = CandidateRootPool::open(&path, vec![first.clone(), second.clone()])?;
    }
    assert!(matches!(
        CandidateRootPool::open(&path, vec![first.clone(), root(3)]),
        Err(PoolError::InvalidCatalog)
    ));
    assert!(matches!(
        CandidateRootPool::open(&path, vec![first.clone()]),
        Err(PoolError::InvalidCatalog)
    ));
    assert!(matches!(
        CandidateRootPool::open(&path, vec![first.clone(), second.clone(), root(3)]),
        Err(PoolError::InvalidCatalog)
    ));
    let _reopened = CandidateRootPool::open(&path, vec![second, first])?;
    Ok(())
}

#[test]
fn claimed_root_does_not_hide_drift_of_an_unclaimed_root() -> Result<(), PoolError> {
    let path = database();
    let first = root(1);
    let second = root(2);
    let claim = {
        let mut pool = CandidateRootPool::open(&path, vec![first.clone(), second.clone()])?;
        pool.reserve(request(), witness(&first))?
    };
    assert!(matches!(
        CandidateRootPool::open(&path, vec![first.clone(), root(3)]),
        Err(PoolError::InvalidCatalog)
    ));
    let reopened = CandidateRootPool::open(&path, vec![first, second])?;
    assert_eq!(reopened.reconcile(&request())?, Some(claim));
    Ok(())
}

#[test]
fn ambiguous_legacy_claims_without_a_catalog_seal_refuse_closed() -> Result<(), PoolError> {
    let path = database();
    let approved = root(1);
    {
        let mut pool = CandidateRootPool::open(&path, vec![approved.clone()])?;
        pool.reserve(request(), witness(&approved))?;
    }
    let database = rusqlite::Connection::open(&path).map_err(|_| PoolError::Storage)?;
    database
        .execute("DELETE FROM approved_root_catalog WHERE singleton = 1", [])
        .map_err(|_| PoolError::Storage)?;
    assert!(matches!(
        CandidateRootPool::open(&path, vec![approved]),
        Err(PoolError::InvalidCatalog)
    ));
    Ok(())
}
