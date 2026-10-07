// SPDX-License-Identifier: MIT

use super::process_lifecycle_fake::FakeProcess;
use super::{
    ApprovedLaunchProfileAdapter, ApprovedLaunchProfiles, AuthorityEpoch, CallerId,
    DeterministicLeaseDecision, InMemoryLifecycleStore, InstanceId, LaunchProfile, LaunchProfileId,
    LaunchSpec, Lease, LeaseEpoch, LifecycleOperationState, LifecycleRequest, ProcessFault,
    ProcessHandle, ProcessLifecycle, ProcessLifecycleConfig, ProcessPort, SessionId, StopMode,
    Tick, UserDataConfig,
};

#[derive(Clone, Copy, Debug)]
struct FakeClock {
    now: Tick,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            now: Tick::from_millis(0),
        }
    }
}

impl super::Clock for FakeClock {
    fn now(&self) -> Tick {
        self.now
    }
}

fn lease_for(instance: u64) -> Lease {
    Lease::new(
        InstanceId::new(instance),
        CallerId::new(11),
        SessionId::new(13),
        super::LeaseId::new(17),
        LeaseEpoch::new(1),
        Tick::from_millis(10_000),
    )
}

fn lease() -> Lease {
    lease_for(7)
}

fn profile(id: u64, image: u64) -> Result<LaunchProfile, String> {
    let executable =
        ExecutableIdentity::try_new(101, 202, image).map_err(|error| format!("{error:?}"))?;
    let user_data = UserDataConfig::try_new(303).map_err(|error| format!("{error:?}"))?;
    let policy =
        super::ProcessPolicy::try_new(2, 1_000, 1_000).map_err(|error| format!("{error:?}"))?;
    LaunchProfile::try_new(LaunchProfileId::new(id), executable, user_data, policy)
        .map_err(|error| format!("{error:?}"))
}

fn profiles() -> Result<ApprovedLaunchProfiles, String> {
    let mut profiles = ApprovedLaunchProfiles::try_new(2).map_err(|error| format!("{error:?}"))?;
    profiles
        .insert(profile(1, 404)?)
        .map_err(|error| format!("{error:?}"))?;
    profiles
        .insert(profile(2, 405)?)
        .map_err(|error| format!("{error:?}"))?;
    Ok(profiles)
}

fn new_lifecycle(
    process: FakeProcess,
    store: InMemoryLifecycleStore,
) -> Result<
    ProcessLifecycle<FakeClock, FakeProcess, InMemoryLifecycleStore, DeterministicLeaseDecision>,
    String,
> {
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(2).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        process,
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(lease())
        .map_err(|error| error.to_string())?;
    Ok(lifecycle)
}

fn launch_request(operation: u64) -> LifecycleRequest {
    LifecycleRequest::launch_new(
        super::OperationId::new(operation),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    )
}

fn launch_request_for(operation: u64, instance: u64, profile_id: u64) -> LifecycleRequest {
    LifecycleRequest::launch_new(
        super::OperationId::new(operation),
        lease_for(instance).proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(profile_id),
    )
}

#[test]
fn approved_adapter_requires_a_server_profile_and_rejects_unapproved_ids() -> Result<(), String> {
    let profiles = profiles()?;
    let mut adapter = ApprovedLaunchProfileAdapter::new(profiles, FakeProcess::default());
    assert_eq!(
        adapter.start(LaunchSpec::new(InstanceId::new(7))),
        Err(ProcessFault::ProfileRequired)
    );
    assert_eq!(
        adapter.start(LaunchSpec::for_profile(
            InstanceId::new(7),
            LaunchProfileId::new(99)
        )),
        Err(ProcessFault::ProfileNotApproved)
    );
    let unapproved = profile(99, 404)?;
    assert_eq!(
        adapter.start_with_profile(
            LaunchSpec::for_profile(InstanceId::new(7), unapproved.id()),
            unapproved,
        ),
        Err(ProcessFault::ProfileNotApproved)
    );
    let handle = adapter
        .start(LaunchSpec::for_profile(
            InstanceId::new(7),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| format!("{error:?}"))?;
    let identity = adapter
        .inspect_identity(handle)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(identity.instance_id(), InstanceId::new(7));
    assert_eq!(adapter.process().starts(), 1);
    Ok(())
}

#[test]
fn approved_adapter_reports_cleanup_failure_for_a_mismatched_launch() -> Result<(), String> {
    let profiles = profiles()?;
    let mut adapter = ApprovedLaunchProfileAdapter::new(profiles, FakeProcess::default());
    adapter.process_mut().set_wrong_identity(true);
    adapter
        .process_mut()
        .set_stop_fault(Some(ProcessFault::StopFailed));
    assert_eq!(
        adapter.start(LaunchSpec::for_profile(
            InstanceId::new(7),
            LaunchProfileId::new(1),
        )),
        Err(ProcessFault::StopFailed)
    );
    adapter.process_mut().set_stop_fault(None);
    adapter
        .stop(ProcessHandle::new(1), StopMode::Force)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(adapter.process().starts(), 1);
    Ok(())
}

#[test]
fn duplicate_launch_replays_the_record_without_a_second_start() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let request = launch_request(1);
    let first = lifecycle
        .apply(request.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(first.state(), super::LifecycleState::Starting);
    assert_eq!(first.operation_state(), LifecycleOperationState::Started);
    assert_eq!(lifecycle.process().starts(), 1);
    let replay = lifecycle
        .apply(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.state(), super::LifecycleState::Starting);
    assert_eq!(replay.operation_state(), LifecycleOperationState::Started);
    assert_eq!(lifecycle.process().starts(), 1);
    Ok(())
}

#[test]
fn attach_requires_a_prior_gateway_identity_and_reuses_it_after_reconnect() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| "missing process".to_owned())?;
    let (process, store) = lifecycle.into_parts();
    let mut restarted = new_lifecycle(process, store)?;
    let request = LifecycleRequest::attach_existing(
        super::OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        identity,
    );
    let attached = restarted
        .apply(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(attached.state(), super::LifecycleState::Starting);
    assert_eq!(
        attached.operation_state(),
        LifecycleOperationState::Attached
    );
    assert_eq!(restarted.process().starts(), 1);
    Ok(())
}

#[path = "process_lifecycle_recovery_tests.rs"]
mod recovery_tests;

#[path = "process_lifecycle_restart_fence_tests.rs"]
mod restart_fence_tests;

#[path = "process_lifecycle_contract_tests.rs"]
mod contract_tests;

#[path = "process_lifecycle_review_tests.rs"]
mod review_tests;

#[path = "process_lifecycle_retained_cleanup_tests.rs"]
mod retained_cleanup_tests;

#[path = "process_lifecycle_disk_tests.rs"]
mod disk_tests;
