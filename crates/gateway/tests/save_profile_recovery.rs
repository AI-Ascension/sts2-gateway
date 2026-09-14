// SPDX-License-Identifier: MIT

//! Crash/restart recovery for isolated user-data provisioning (gateway issue #51).
//!
//! The provisioner persists an intent record before it touches the injected allocation port.
//! These tests reopen a persisted store as a fresh provisioner and prove that the same opaque
//! identity is recovered without a second allocation, for both a crash before the port write and a
//! crash after the host may have accepted the write.

use sts2_gateway::{
    InMemoryLaunchProfileBindingPort, InMemoryUserDataPort, InMemoryUserDataRecordStore,
    LAUNCH_PROFILE_CONTRACT, LAUNCH_PROFILE_ID, LaunchProfileBinding, SaveProfileContext,
    UserDataCreateOutcome, UserDataDescriptor, UserDataIdentity, UserDataInspection,
    UserDataProvenance, UserDataProvisioner, UserDataProvisioningRecord,
    UserDataProvisioningStatus, UserDataRecordStore,
};

const INSTANCE: &str = "instance-1";

fn context(correlation: &str) -> SaveProfileContext {
    SaveProfileContext {
        instance_id: INSTANCE.to_owned(),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        correlation_id: correlation.to_owned(),
    }
}

fn descriptor(identity: u64, operation_id: &str) -> UserDataDescriptor {
    UserDataDescriptor {
        identity: UserDataIdentity::new(identity),
        provenance: UserDataProvenance {
            owner: String::from("gateway"),
            instance_id: INSTANCE.to_owned(),
            operation_id: operation_id.to_owned(),
            contract: LAUNCH_PROFILE_CONTRACT.to_owned(),
        },
        baseline: None,
    }
}

fn binding(identity: u64) -> Result<LaunchProfileBinding, String> {
    LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, UserDataIdentity::new(identity))
        .map_err(|error| format!("{error:?}"))
}

#[test]
fn reopened_pending_allocation_recovers_identity_without_second_create() -> Result<(), String> {
    let context = context("recover-pending");
    // A crash after the intent insert but before the port create leaves a Pending record.
    let mut store = InMemoryUserDataRecordStore::default();
    store
        .insert(UserDataProvisioningRecord {
            operation_id: String::from("op-1"),
            context: context.clone(),
            descriptor: descriptor(1, "op-1"),
            launch_profile: binding(1)?,
            status: UserDataProvisioningStatus::Pending,
            guidance: None,
        })
        .map_err(|error| format!("{error:?}"))?;

    let mut provisioner = UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        store,
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;

    let outcome = provisioner
        .reconcile(&context, "op-1")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(outcome.status, UserDataProvisioningStatus::Unknown);
    assert_eq!(outcome.operation_id, "op-1");
    assert!(outcome.descriptor.is_none());
    assert!(
        provisioner.port().created.is_empty(),
        "reconcile must not re-run a pending allocation"
    );
    assert_eq!(
        provisioner
            .operation(INSTANCE, "op-1")
            .map(|record| record.descriptor.identity),
        Some(UserDataIdentity::new(1)),
        "the reserved identity must be retained across the restart"
    );

    // A later, unrelated allocation must not reuse the retained identity.
    let next = provisioner
        .create_disposable(context.clone(), "op-2", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(next.status, UserDataProvisioningStatus::Created);
    assert_eq!(
        next.descriptor.map(|value| value.identity),
        Some(UserDataIdentity::new(2))
    );
    Ok(())
}

#[test]
fn reopened_unknown_allocation_reconciles_to_created_without_duplicate() -> Result<(), String> {
    let context = context("recover-unknown");
    let mut port = InMemoryUserDataPort::default();
    // The host may have accepted the write even though the reply was lost.
    port.set_outcome(
        UserDataIdentity::new(1),
        UserDataCreateOutcome::TimeoutAfterWrite,
    );
    let mut provisioner = UserDataProvisioner::new(
        4,
        port,
        InMemoryUserDataRecordStore::default(),
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let first = provisioner
        .create_disposable(context.clone(), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(first.status, UserDataProvisioningStatus::Unknown);
    let store = provisioner.into_store();

    let mut reopened_port = InMemoryUserDataPort::default();
    reopened_port.set_inspection(
        UserDataIdentity::new(1),
        UserDataInspection::Owned(descriptor(1, "op-1").provenance),
    );
    let mut reopened = UserDataProvisioner::new(
        4,
        reopened_port,
        store,
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;

    let outcome = reopened
        .reconcile(&context, "op-1")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(outcome.status, UserDataProvisioningStatus::Created);
    assert_eq!(outcome.operation_id, "op-1");
    assert_eq!(
        outcome.descriptor.as_ref().map(|value| value.identity),
        Some(UserDataIdentity::new(1)),
        "the accepted allocation identity must be recovered, not repeated"
    );
    assert!(
        reopened.port().created.is_empty(),
        "reconcile must not repeat a possibly-accepted create"
    );

    // A fresh allocation after recovery still gets the next unused identity.
    let next = reopened
        .create_disposable(context.clone(), "op-2", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        next.descriptor.map(|value| value.identity),
        Some(UserDataIdentity::new(2))
    );
    Ok(())
}
