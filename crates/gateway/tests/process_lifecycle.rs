// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use sts2_gateway::{
    ApprovedLaunchProfiles, AuthorityEpoch, Clock, DeterministicLeaseDecision, ExecutableIdentity,
    InMemoryLifecycleStore, InstanceId, LaunchProfile, LaunchProfileId, Lease, LifecycleError,
    LifecycleOperationState, LifecycleRecordStore, LifecycleRequest, ProcessDescendantIdentity,
    ProcessFault, ProcessHandle, ProcessIdentity, ProcessLaunch, ProcessLifecycle,
    ProcessLifecycleConfig, ProcessPolicy, ProcessPort, ProcessState, StopMode, UserDataConfig,
};

#[allow(dead_code)]
mod support;

#[derive(Clone)]
struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> sts2_gateway::Tick {
        sts2_gateway::Tick::from_millis(0)
    }
}

struct Entry {
    identity: ProcessIdentity,
    state: ProcessState,
    descendants: Vec<ProcessDescendantIdentity>,
}

#[derive(Default)]
struct ProcessStateStore {
    next_handle: u64,
    entries: BTreeMap<ProcessHandle, Entry>,
    starts: usize,
    stops: usize,
    fail_stop: bool,
    wrong_image: bool,
    recover: bool,
    descendants_after_stop: Vec<ProcessDescendantIdentity>,
}

#[derive(Clone, Default)]
struct RecordingProcess(Rc<RefCell<ProcessStateStore>>);

impl RecordingProcess {
    fn starts(&self) -> usize {
        self.0.borrow().starts
    }

    fn stops(&self) -> usize {
        self.0.borrow().stops
    }

    fn set_fail_stop(&self, value: bool) {
        self.0.borrow_mut().fail_stop = value;
    }

    fn set_wrong_image(&self, value: bool) {
        self.0.borrow_mut().wrong_image = value;
    }

    fn set_recover(&self, value: bool) {
        self.0.borrow_mut().recover = value;
    }

    fn set_descendants_after_stop(&self, descendants: Vec<ProcessDescendantIdentity>) {
        self.0.borrow_mut().descendants_after_stop = descendants;
    }

    fn crash(&self, process: ProcessHandle) {
        if let Some(entry) = self.0.borrow_mut().entries.get_mut(&process) {
            entry.state = ProcessState::Exited { code: Some(17) };
        }
    }
}

impl ProcessPort for RecordingProcess {
    fn start(
        &mut self,
        _specification: sts2_gateway::LaunchSpec,
    ) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn start_with_profile(
        &mut self,
        specification: sts2_gateway::LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        let mut state = self.0.borrow_mut();
        state.next_handle = state.next_handle.saturating_add(1);
        state.starts += 1;
        let handle = ProcessHandle::new(state.next_handle);
        let executable = if state.wrong_image {
            ExecutableIdentity::new(91, 92, 93)
        } else {
            profile.executable()
        };
        let identity = ProcessIdentity::new(
            specification.instance_id(),
            handle,
            1000 + handle.value(),
            5000 + handle.value(),
            executable,
            profile.user_data(),
        );
        state.entries.insert(
            handle,
            Entry {
                identity: identity.clone(),
                state: ProcessState::Running,
                descendants: Vec::new(),
            },
        );
        Ok(ProcessLaunch::new(identity))
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        self.0
            .borrow()
            .entries
            .get(&process)
            .map(|entry| entry.state)
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        self.0
            .borrow()
            .entries
            .get(&process)
            .map(|entry| entry.identity.clone())
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn stop(&mut self, process: ProcessHandle, _mode: StopMode) -> Result<(), ProcessFault> {
        let mut state = self.0.borrow_mut();
        state.stops += 1;
        if state.fail_stop {
            return Err(ProcessFault::StopFailed);
        }
        let survivors = state.descendants_after_stop.clone();
        let Some(entry) = state.entries.get_mut(&process) else {
            return Err(ProcessFault::InspectionFailed);
        };
        entry.state = ProcessState::Exited { code: None };
        entry.descendants = survivors;
        Ok(())
    }

    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        self.0
            .borrow()
            .entries
            .get(&process)
            .map(|entry| entry.descendants.clone())
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn recover_owned(
        &mut self,
        instance_id: InstanceId,
        profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        let state = self.0.borrow();
        if !state.recover {
            return Ok(None);
        }
        Ok(state
            .entries
            .values()
            .find(|entry| {
                entry.state == ProcessState::Running
                    && entry.identity.matches_profile(instance_id, profile)
            })
            .map(|entry| entry.identity.clone()))
    }
}

fn allocations() -> Result<(Lease, Lease), String> {
    let (mut gateway, _clock, _process, _readiness, _transport) = support::new_gateway();
    let first = gateway
        .allocate(support::owner(), support::session())
        .map_err(|error| error.to_string())?;
    let second = gateway
        .allocate(
            sts2_gateway::CallerId::new(8),
            sts2_gateway::SessionId::new(12),
        )
        .map_err(|error| error.to_string())?;
    Ok((first.lease(), second.lease()))
}

fn profile_catalog() -> Result<(ApprovedLaunchProfiles, LaunchProfile), String> {
    let id = LaunchProfileId::try_new(1).map_err(|error| format!("{error:?}"))?;
    let executable =
        ExecutableIdentity::try_new(11, 22, 33).map_err(|error| format!("{error:?}"))?;
    let user_data = UserDataConfig::try_new(44).map_err(|error| format!("{error:?}"))?;
    let policy = ProcessPolicy::try_new(4, 1000, 1000).map_err(|error| format!("{error:?}"))?;
    let profile = LaunchProfile::try_new(id, executable, user_data, policy)
        .map_err(|error| format!("{error:?}"))?;
    let mut catalog = ApprovedLaunchProfiles::try_new(4).map_err(|error| format!("{error:?}"))?;
    catalog
        .insert(profile)
        .map_err(|error| format!("{error:?}"))?;
    Ok((catalog, profile))
}

fn manager(
    process: RecordingProcess,
    store: InMemoryLifecycleStore,
    lease: Lease,
    capacity: usize,
) -> Result<
    ProcessLifecycle<
        TestClock,
        RecordingProcess,
        InMemoryLifecycleStore,
        DeterministicLeaseDecision,
    >,
    String,
> {
    let (catalog, _) = profile_catalog()?;
    let config = ProcessLifecycleConfig::try_new(capacity).map_err(|error| error.to_string())?;
    let mut lifecycle = ProcessLifecycle::new(
        config,
        catalog,
        TestClock,
        process,
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(lease)
        .map_err(|error| error.to_string())?;
    Ok(lifecycle)
}

#[test]
fn launch_is_idempotent_and_attach_requires_the_retained_identity() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 2)?;
    let profile_id = LaunchProfileId::new(1);
    let request = LifecycleRequest::launch_new(
        sts2_gateway::OperationId::new(1),
        lease.proof(),
        AuthorityEpoch::new(1),
        profile_id,
    );
    let first = lifecycle
        .apply(request.clone())
        .map_err(|error| error.to_string())?;
    let second = lifecycle
        .apply(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(process.starts(), 1);
    assert_eq!(first.process(), second.process());
    assert_eq!(first.state(), sts2_gateway::LifecycleState::Starting);
    let identity = first.process().cloned().ok_or("missing process identity")?;
    let attached = lifecycle
        .apply(LifecycleRequest::attach_existing(
            sts2_gateway::OperationId::new(2),
            lease.proof(),
            AuthorityEpoch::new(1),
            identity.clone(),
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        attached.operation_state(),
        LifecycleOperationState::Attached
    );
    assert_eq!(attached.process(), Some(&identity));
    Ok(())
}

#[test]
fn stale_epoch_and_unowned_attach_have_no_process_effect() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    let profile = profile_catalog()?.1;
    let foreign = ProcessIdentity::new(
        lease.instance_id(),
        ProcessHandle::new(90),
        90,
        900,
        profile.executable(),
        profile.user_data(),
    );
    assert_eq!(
        lifecycle.apply(LifecycleRequest::attach_existing(
            sts2_gateway::OperationId::new(1),
            lease.proof(),
            AuthorityEpoch::new(0),
            foreign.clone(),
        )),
        Err(LifecycleError::StaleAuthorityEpoch)
    );
    assert_eq!(
        lifecycle.apply(LifecycleRequest::attach_existing(
            sts2_gateway::OperationId::new(1),
            lease.proof(),
            AuthorityEpoch::new(1),
            foreign,
        )),
        Err(LifecycleError::UnownedAttach)
    );
    assert_eq!(process.starts(), 0);
    assert_eq!(process.stops(), 0);
    Ok(())
}

#[test]
fn wrong_image_stop_failure_and_descendant_survival_fail_closed() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    process.set_wrong_image(true);
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    assert_eq!(
        lifecycle.apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(1),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        )),
        Err(LifecycleError::IdentityMismatch)
    );
    assert_eq!(process.stops(), 1);

    process.set_wrong_image(false);
    let started = lifecycle
        .apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(2),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    let identity = started.process().cloned().ok_or("missing identity")?;
    process.set_fail_stop(true);
    assert_eq!(
        lifecycle.apply(LifecycleRequest::stop(
            sts2_gateway::OperationId::new(3),
            lease.proof(),
            AuthorityEpoch::new(1),
            StopMode::Force,
        )),
        Err(LifecycleError::Process(ProcessFault::StopFailed))
    );
    process.set_fail_stop(false);
    process.set_descendants_after_stop(vec![ProcessDescendantIdentity::new(
        lease.instance_id(),
        700,
        701,
        identity.executable().ok_or("missing executable")?,
        identity.user_data().ok_or("missing user data")?,
    )]);
    assert_eq!(
        lifecycle.apply(LifecycleRequest::stop(
            sts2_gateway::OperationId::new(4),
            lease.proof(),
            AuthorityEpoch::new(1),
            StopMode::Force,
        )),
        Err(LifecycleError::ForeignDescendant)
    );
    Ok(())
}

#[test]
fn restart_rotates_authority_and_rejects_the_old_epoch() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    let started = lifecycle
        .apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(1),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    assert!(started.process().is_some());
    let restarted = lifecycle
        .apply(LifecycleRequest::restart(
            sts2_gateway::OperationId::new(2),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(restarted.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(process.starts(), 2);
    assert_eq!(
        lifecycle.apply(LifecycleRequest::stop(
            sts2_gateway::OperationId::new(3),
            lease.proof(),
            AuthorityEpoch::new(1),
            StopMode::Force,
        )),
        Err(LifecycleError::StaleAuthorityEpoch)
    );
    Ok(())
}

#[path = "process_lifecycle_recovery/mod.rs"]
mod recovery_tests;
