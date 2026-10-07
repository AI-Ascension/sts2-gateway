// SPDX-License-Identifier: MIT

use crate::{
    ApprovedLaunchProfileAdapter, ApprovedLaunchProfiles, AuthorityEpoch, CallerId,
    DeterministicLeaseDecision, ExecutableIdentity, GenerationRecovery, GenerationStartError,
    InMemoryLifecycleStore, InstanceId, LaunchProfile, LaunchProfileId, LaunchSpec, Lease,
    LeaseEpoch, LeaseId, LeaseProof, LifecycleAction, LifecycleError, LifecycleOperation,
    LifecycleOperationState, LifecycleOwnership, LifecycleRecordStore, LifecycleRequest,
    OperationId, ProcessFault, ProcessGenerationKind, ProcessHandle, ProcessIdentity,
    ProcessLaunch, ProcessLifecycle, ProcessLifecycleConfig, ProcessOperationGeneration,
    ProcessPolicy, ProcessPort, ProcessState, SessionId, StopMode, Tick, UserDataConfig,
};

fn test_error(context: &str, error: impl std::fmt::Debug) -> String {
    format!("{context}: {error:?}")
}

fn profile(id: u64) -> Result<LaunchProfile, String> {
    let user_data =
        UserDataConfig::try_new(44).map_err(|error| test_error("valid user-data config", error))?;
    let policy = ProcessPolicy::try_new(1, 100, 100)
        .map_err(|error| test_error("valid process policy", error))?;
    LaunchProfile::try_new(
        LaunchProfileId::new(id),
        ExecutableIdentity::new(11, 22, 33),
        user_data,
        policy,
    )
    .map_err(|error| test_error("valid launch profile", error))
}

#[derive(Clone, Copy)]
struct GenerationTestClock;

impl crate::Clock for GenerationTestClock {
    fn now(&self) -> Tick {
        Tick::from_millis(0)
    }
}

fn lease(instance: u64) -> Lease {
    Lease::new(
        InstanceId::new(instance),
        CallerId::new(2),
        SessionId::new(3),
        LeaseId::new(4),
        LeaseEpoch::new(1),
        Tick::from_millis(1000),
    )
}

fn operation(
    instance: u64,
    operation: u64,
    sequence: u64,
    request_epoch: u64,
    authority_epoch: u64,
    profile_id: u64,
    kind: ProcessGenerationKind,
) -> LifecycleOperation {
    let instance_id = InstanceId::new(instance);
    let lease = LeaseProof::new(
        instance_id,
        CallerId::new(2),
        SessionId::new(3),
        LeaseId::new(4),
        LeaseEpoch::new(1),
    );
    let action = match kind {
        ProcessGenerationKind::LaunchNew => LifecycleAction::LaunchNew {
            profile_id: LaunchProfileId::new(profile_id),
        },
        ProcessGenerationKind::RestartReplacement => LifecycleAction::Restart {
            profile_id: LaunchProfileId::new(profile_id),
        },
    };
    let mut persisted = LifecycleOperation::new(
        OperationId::new(operation),
        instance_id,
        lease,
        AuthorityEpoch::new(request_epoch),
        action,
    );
    persisted.set_sequence(sequence);
    persisted.set_authority_epoch(AuthorityEpoch::new(authority_epoch));
    persisted.set_state(
        match kind {
            ProcessGenerationKind::LaunchNew => LifecycleOperationState::Starting,
            ProcessGenerationKind::RestartReplacement => LifecycleOperationState::Restarting,
        },
        None,
        None,
    );
    persisted
}

fn generation(
    instance: u64,
    operation_id: u64,
    sequence: u64,
    request_epoch: u64,
    authority_epoch: u64,
    profile_id: u64,
    kind: ProcessGenerationKind,
) -> Result<ProcessOperationGeneration, String> {
    let profile = profile(profile_id)?;
    ProcessOperationGeneration::for_operation(
        &operation(
            instance,
            operation_id,
            sequence,
            request_epoch,
            authority_epoch,
            profile_id,
            kind,
        ),
        profile,
        kind,
    )
    .ok_or_else(|| "operation does not authorize this generation".to_owned())
}

#[test]
fn generation_key_contains_every_operation_and_fence_component() -> Result<(), String> {
    let kind = ProcessGenerationKind::LaunchNew;
    let key = generation(7, 9, 12, 3, 3, 2, kind)?;
    assert_eq!(key.instance_id(), InstanceId::new(7));
    assert_eq!(key.operation_id(), OperationId::new(9));
    assert_eq!(key.sequence(), 12);
    assert_eq!(key.request_epoch(), AuthorityEpoch::new(3));
    assert_eq!(key.authority_epoch(), AuthorityEpoch::new(3));
    assert_eq!(key.profile_id(), LaunchProfileId::new(2));
    assert_eq!(key.kind(), kind);

    for different in [
        generation(8, 9, 12, 3, 3, 2, kind)?,
        generation(7, 10, 12, 3, 3, 2, kind)?,
        generation(7, 9, 13, 3, 3, 2, kind)?,
        generation(7, 9, 12, 4, 3, 2, kind)?,
        generation(7, 9, 12, 3, 4, 2, kind)?,
        generation(7, 9, 12, 3, 3, 3, kind)?,
        generation(7, 9, 12, 3, 4, 2, ProcessGenerationKind::RestartReplacement)?,
    ] {
        assert_ne!(&key, &different);
    }
    Ok(())
}

#[test]
fn legacy_zero_sequence_and_unfenced_restart_cannot_create_generation_keys() -> Result<(), String> {
    assert!(generation(7, 9, 0, 3, 3, 2, ProcessGenerationKind::LaunchNew).is_err());
    assert!(generation(7, 9, 12, 3, 3, 2, ProcessGenerationKind::RestartReplacement,).is_err());
    Ok(())
}

#[derive(Default)]
struct LegacyOnlyPort {
    legacy_starts: usize,
    legacy_recoveries: usize,
}

impl ProcessPort for LegacyOnlyPort {
    fn start(&mut self, _specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn inspect(&mut self, _process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        Err(ProcessFault::InspectionFailed)
    }

    fn stop(&mut self, _process: ProcessHandle, _mode: StopMode) -> Result<(), ProcessFault> {
        Ok(())
    }

    fn start_with_profile(
        &mut self,
        _specification: LaunchSpec,
        _profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        self.legacy_starts += 1;
        Err(ProcessFault::StartRejected)
    }

    fn recover_owned(
        &mut self,
        _instance_id: InstanceId,
        _profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        self.legacy_recoveries += 1;
        Ok(None)
    }
}

#[test]
fn generation_defaults_fail_closed_without_calling_legacy_methods() -> Result<(), String> {
    let profile = profile(2)?;
    let key = generation(7, 9, 12, 3, 3, 2, ProcessGenerationKind::LaunchNew)?;
    let mut port = LegacyOnlyPort::default();

    assert_eq!(
        port.start_generation(
            &key,
            LaunchSpec::for_profile(InstanceId::new(7), profile.id()),
            profile,
        ),
        Err(GenerationStartError::Unsupported)
    );
    assert_eq!(
        port.recover_generation(&key, profile),
        Ok(GenerationRecovery::Indeterminate)
    );
    assert_eq!(port.legacy_starts, 0);
    assert_eq!(port.legacy_recoveries, 0);
    Ok(())
}

#[test]
fn unsupported_start_releases_reservation_because_it_guarantees_no_effect() -> Result<(), String> {
    let selected_profile = profile(2)?;
    let mut catalog =
        ApprovedLaunchProfiles::try_new(1).map_err(|error| test_error("valid catalog", error))?;
    catalog
        .insert(selected_profile)
        .map_err(|error| test_error("profile accepted", error))?;
    let owner = lease(7);
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1)
            .map_err(|error| test_error("valid lifecycle config", error))?,
        catalog,
        GenerationTestClock,
        LegacyOnlyPort::default(),
        InMemoryLifecycleStore::new(),
        DeterministicLeaseDecision,
    )
    .map_err(|error| test_error("lifecycle constructed", error))?;
    lifecycle
        .bind_lease(owner)
        .map_err(|error| test_error("lease bound", error))?;
    let request = |operation_id| {
        LifecycleRequest::launch_new(
            OperationId::new(operation_id),
            owner.proof(),
            AuthorityEpoch::new(1),
            selected_profile.id(),
        )
    };

    assert_eq!(
        lifecycle.apply(request(9)),
        Err(LifecycleError::Process(ProcessFault::ProfileRequired))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(9))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Failed)
    );
    assert_eq!(
        lifecycle.apply(request(10)),
        Err(LifecycleError::Process(ProcessFault::ProfileRequired))
    );
    assert_eq!(lifecycle.process().legacy_starts, 0);
    assert_eq!(lifecycle.process().legacy_recoveries, 0);
    Ok(())
}

#[test]
fn approved_adapter_forwards_the_complete_generation_key() -> Result<(), String> {
    let profile = profile(2)?;
    let mut catalog =
        ApprovedLaunchProfiles::try_new(1).map_err(|error| test_error("valid catalog", error))?;
    catalog
        .insert(profile)
        .map_err(|error| test_error("profile accepted", error))?;
    let key = generation(7, 9, 12, 3, 3, 2, ProcessGenerationKind::LaunchNew)?;
    let mut adapter = ApprovedLaunchProfileAdapter::new(
        catalog,
        crate::process_lifecycle_fake::FakeProcess::default(),
    );

    let started = adapter
        .start_generation(
            &key,
            LaunchSpec::for_profile(InstanceId::new(7), profile.id()),
            profile,
        )
        .map_err(|error| test_error("generation start forwarded", error))?;
    adapter.process_mut().set_recover_enabled(true);
    assert_eq!(
        adapter.recover_generation(&key, profile),
        Ok(GenerationRecovery::Found(started.identity().clone()))
    );
    assert_eq!(adapter.process().starts(), 1);
    Ok(())
}

#[test]
fn fake_recovery_isolated_by_operation_id_and_reuses_the_same_key() -> Result<(), String> {
    let profile = profile(2)?;
    let first = generation(7, 9, 12, 3, 3, 2, ProcessGenerationKind::LaunchNew)?;
    let other = generation(7, 10, 13, 3, 3, 2, ProcessGenerationKind::LaunchNew)?;
    let mut fake = crate::process_lifecycle_fake::FakeProcess::default();
    let started = fake
        .start_generation(
            &first,
            LaunchSpec::for_profile(InstanceId::new(7), profile.id()),
            profile,
        )
        .map_err(|error| test_error("generation start succeeds", error))?;
    fake.set_recover_enabled(true);

    assert_eq!(
        fake.recover_generation(&other, profile),
        Ok(GenerationRecovery::Indeterminate)
    );
    assert_eq!(
        fake.recover_generation(&first, profile),
        Ok(GenerationRecovery::Found(started.identity().clone()))
    );
    assert_eq!(fake.starts(), 1);
    assert_eq!(fake.generation_queries(), &[other, first]);
    Ok(())
}

#[test]
fn identityless_unrotated_or_stale_restart_never_queries_the_adapter() -> Result<(), String> {
    for (request_epoch, authority_epoch, current_epoch) in [(1, 1, 1), (1, 2, 3)] {
        let selected_profile = profile(2)?;
        let mut operation = LifecycleOperation::new(
            OperationId::new(9),
            InstanceId::new(7),
            lease(7).proof(),
            AuthorityEpoch::new(request_epoch),
            LifecycleAction::Restart {
                profile_id: selected_profile.id(),
            },
        );
        operation.set_sequence(12);
        operation.set_authority_epoch(AuthorityEpoch::new(authority_epoch));
        operation.set_state(LifecycleOperationState::Restarting, None, None);

        let mut store = InMemoryLifecycleStore::new();
        store
            .insert(operation.clone())
            .map_err(|error| test_error("restart record stored", error))?;
        store
            .set_ownership(LifecycleOwnership::new(
                InstanceId::new(7),
                OperationId::new(9),
                12,
                selected_profile.id(),
                None,
            ))
            .map_err(|error| test_error("identity-less reservation stored", error))?;
        let mut catalog = ApprovedLaunchProfiles::try_new(1)
            .map_err(|error| test_error("valid catalog", error))?;
        catalog
            .insert(selected_profile)
            .map_err(|error| test_error("profile accepted", error))?;
        let mut lifecycle = ProcessLifecycle::new(
            ProcessLifecycleConfig::try_new(1)
                .map_err(|error| test_error("valid lifecycle config", error))?,
            catalog,
            GenerationTestClock,
            crate::process_lifecycle_fake::FakeProcess::default(),
            store,
            DeterministicLeaseDecision,
        )
        .map_err(|error| test_error("lifecycle constructed", error))?;
        lifecycle
            .bind_lease(lease(7))
            .map_err(|error| test_error("lease bound", error))?;
        lifecycle.set_authority_epoch(InstanceId::new(7), AuthorityEpoch::new(current_epoch));

        let response = lifecycle
            .reconcile_restart(operation)
            .map_err(|error| test_error("uncertain restart remains observable", error))?;
        assert_eq!(response.operation_state(), LifecycleOperationState::Unknown);
        assert_eq!(lifecycle.process().starts(), 0);
        assert!(lifecycle.process().generation_queries().is_empty());
        assert!(lifecycle.ownership.contains_key(&InstanceId::new(7)));
    }
    Ok(())
}

#[test]
fn retained_adapter_cleanup_is_exact_keyed_and_forgotten_after_stop() -> Result<(), String> {
    let profile = profile(2)?;
    let mut catalog =
        ApprovedLaunchProfiles::try_new(1).map_err(|error| test_error("valid catalog", error))?;
    catalog
        .insert(profile)
        .map_err(|error| test_error("profile accepted", error))?;
    let key = generation(7, 9, 12, 3, 3, 2, ProcessGenerationKind::LaunchNew)?;
    let other = generation(7, 10, 13, 3, 3, 2, ProcessGenerationKind::LaunchNew)?;
    let mut fake = crate::process_lifecycle_fake::FakeProcess::default();
    fake.set_wrong_image(true);
    fake.set_stop_fault(Some(ProcessFault::StopFailed));
    let mut adapter = ApprovedLaunchProfileAdapter::new(catalog, fake);
    assert_eq!(
        adapter.start_generation(
            &key,
            LaunchSpec::for_profile(InstanceId::new(7), profile.id()),
            profile,
        ),
        Err(GenerationStartError::Process(ProcessFault::StopFailed))
    );
    assert_eq!(
        adapter.recover_generation(&other, profile),
        Ok(GenerationRecovery::Indeterminate)
    );
    let GenerationRecovery::Found(identity) = adapter
        .recover_generation(&key, profile)
        .map_err(|error| test_error("retained exact-key recovery", error))?
    else {
        return Err("missing exact retained cleanup identity".to_owned());
    };
    assert!(!identity.matches_profile(InstanceId::new(7), profile));
    adapter.process_mut().set_stop_fault(None);
    adapter
        .stop(identity.process(), StopMode::Force)
        .map_err(|error| test_error("exact retained cleanup", error))?;
    assert_eq!(
        adapter.recover_generation(&key, profile),
        Ok(GenerationRecovery::Indeterminate)
    );
    assert_eq!(adapter.process().starts(), 1);
    Ok(())
}
