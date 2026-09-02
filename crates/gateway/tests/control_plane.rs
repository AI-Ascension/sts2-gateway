// SPDX-License-Identifier: MIT

mod support;

use sts2_gateway::{
    FenceFailure, FixedRoute, GatewayError, HealthFault, InstanceId, LeaseEpoch, LeaseProof,
    OperationId, ProcessFault, ProcessState, Readiness, StopMode, TransportFault,
};
use support::{new_gateway, owner, ready_gateway, session};

#[test]
fn allocation_reconciles_through_readiness() -> Result<(), String> {
    let (mut gateway, _clock, process, _readiness, _transport) = new_gateway();
    let allocation = gateway
        .allocate(owner(), session())
        .map_err(|error| error.to_string())?;
    let starting = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(starting.state(), sts2_gateway::LifecycleState::Starting);
    assert_eq!(starting.lease(), Some(allocation.lease()));
    assert!(process.handle_for(allocation.instance_id()).is_some());

    let state = gateway
        .reconcile(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(state, sts2_gateway::LifecycleState::Ready);
    Ok(())
}

#[test]
fn stale_epoch_and_wrong_instance_are_denied_before_transport() -> Result<(), String> {
    let (mut gateway, _clock, _process, _readiness, transport) = new_gateway();
    let first = ready_gateway(&mut gateway)?;
    let second = ready_gateway(&mut gateway)?;
    let stale = LeaseProof::new(
        first.instance_id(),
        owner(),
        session(),
        first.lease().lease_id(),
        LeaseEpoch::new(0),
    );

    let stale_result = gateway.forward(
        first.instance_id(),
        stale,
        OperationId::new(1),
        FixedRoute::Command,
        vec![1],
    );
    assert_eq!(
        stale_result,
        Err(GatewayError::Fence(FenceFailure::StaleEpoch))
    );

    let wrong_instance = gateway.forward(
        second.instance_id(),
        first.lease().proof(),
        OperationId::new(2),
        FixedRoute::ReadOnly,
        vec![1],
    );
    assert_eq!(
        wrong_instance,
        Err(GatewayError::Fence(FenceFailure::WrongInstance))
    );
    assert_eq!(transport.calls(), 0);
    Ok(())
}

#[test]
fn release_then_cleanup_removes_instance() -> Result<(), String> {
    let (mut gateway, _clock, process, _readiness, _transport) = new_gateway();
    let allocation = ready_gateway(&mut gateway)?;
    gateway
        .release(allocation.lease().proof())
        .map_err(|error| error.to_string())?;
    let stopped = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(stopped.state(), sts2_gateway::LifecycleState::Stopped);
    assert!(!stopped.process_attached());
    assert_eq!(process.stop_modes(), vec![StopMode::Graceful]);

    gateway
        .cleanup(allocation.instance_id(), owner(), session())
        .map_err(|error| error.to_string())?;
    assert_eq!(
        gateway.status(allocation.instance_id()),
        Err(GatewayError::InstanceNotFound)
    );
    assert_eq!(process.stop_modes(), vec![StopMode::Graceful]);
    Ok(())
}

#[test]
fn expiry_revokes_lease_and_cleans_process() -> Result<(), String> {
    let (mut gateway, clock, process, _readiness, _transport) = new_gateway();
    let allocation = gateway
        .allocate(owner(), session())
        .map_err(|error| error.to_string())?;
    clock.advance(10);
    let events = gateway.expire_due();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].instance_id(), allocation.instance_id());
    assert_eq!(events[0].status(), sts2_gateway::CleanupStatus::Cleaned);

    let expired = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(expired.state(), sts2_gateway::LifecycleState::Expired);
    assert_eq!(expired.lease(), None);
    assert!(!expired.process_attached());
    assert_eq!(process.stop_modes(), vec![StopMode::Force]);
    gateway
        .cleanup(allocation.instance_id(), owner(), session())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn start_failure_is_visible_and_cleanable() -> Result<(), String> {
    let (mut gateway, _clock, process, _readiness, _transport) = new_gateway();
    process.set_start_fault(Some(ProcessFault::Unavailable));
    let result = gateway.allocate(owner(), session());
    assert_eq!(
        result,
        Err(GatewayError::ProcessStart(ProcessFault::Unavailable))
    );
    let failed = gateway
        .status(InstanceId::new(1))
        .map_err(|error| error.to_string())?;
    assert_eq!(failed.state(), sts2_gateway::LifecycleState::Failed);
    assert_eq!(failed.lease(), None);
    assert!(!failed.process_attached());
    gateway
        .cleanup(InstanceId::new(1), owner(), session())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn readiness_and_crash_fail_closed() -> Result<(), String> {
    let (mut gateway, _clock, process, readiness, _transport) = new_gateway();
    let allocation = gateway
        .allocate(owner(), session())
        .map_err(|error| error.to_string())?;
    process.set_inspect_fault(Some(ProcessFault::InspectionFailed));
    assert_eq!(
        gateway.reconcile(allocation.instance_id()),
        Err(GatewayError::ProcessInspection(
            ProcessFault::InspectionFailed,
        ))
    );
    process.set_inspect_fault(None);
    readiness.set(Err(HealthFault::Unavailable));
    assert_eq!(
        gateway.reconcile(allocation.instance_id()),
        Err(GatewayError::Readiness(HealthFault::Unavailable))
    );
    let degraded = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(degraded.state(), sts2_gateway::LifecycleState::Degraded);

    let Some(process_handle) = process.handle_for(allocation.instance_id()) else {
        return Err("fake process handle was not allocated".to_owned());
    };
    process.set_status(process_handle, ProcessState::Exited { code: Some(17) });
    readiness.set(Ok(Readiness::Ready));
    assert_eq!(
        gateway.reconcile(allocation.instance_id()),
        Err(GatewayError::ProcessCrashed)
    );
    let failed = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(failed.state(), sts2_gateway::LifecycleState::Failed);
    gateway
        .cleanup(allocation.instance_id(), owner(), session())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn shutdown_reports_stop_failure_and_closes_admission() -> Result<(), String> {
    let (mut gateway, _clock, process, _readiness, _transport) = new_gateway();
    let allocation = ready_gateway(&mut gateway)?;
    process.set_stop_fault(Some(ProcessFault::StopFailed));
    let report = gateway.shutdown();
    assert_eq!(report.stopped(), 0);
    assert_eq!(report.failed(), 1);
    assert_eq!(
        gateway.allocate(owner(), session()),
        Err(GatewayError::AdmissionClosed)
    );
    let failed = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(failed.state(), sts2_gateway::LifecycleState::Failed);
    process.set_stop_fault(None);
    gateway
        .cleanup(allocation.instance_id(), owner(), session())
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn fixed_transport_is_bounded_and_fail_closed() -> Result<(), String> {
    let (mut gateway, _clock, _process, _readiness, transport) = new_gateway();
    let allocation = ready_gateway(&mut gateway)?;
    let response = gateway
        .forward(
            allocation.instance_id(),
            allocation.lease().proof(),
            OperationId::new(3),
            FixedRoute::Receipt,
            vec![2, 3],
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(response.status(), 200);
    assert_eq!(transport.calls(), 1);

    assert_eq!(
        gateway.forward(
            allocation.instance_id(),
            allocation.lease().proof(),
            OperationId::new(4),
            FixedRoute::Command,
            vec![0; 9],
        ),
        Err(GatewayError::BodyTooLarge {
            limit: 8,
            actual: 9,
        })
    );
    assert_eq!(transport.calls(), 1);

    transport.set_fault(Some(TransportFault::Disconnected));
    assert_eq!(
        gateway.forward(
            allocation.instance_id(),
            allocation.lease().proof(),
            OperationId::new(5),
            FixedRoute::Command,
            vec![4],
        ),
        Err(GatewayError::Transport(TransportFault::Disconnected))
    );
    let degraded = gateway
        .status(allocation.instance_id())
        .map_err(|error| error.to_string())?;
    assert_eq!(degraded.state(), sts2_gateway::LifecycleState::Degraded);
    Ok(())
}
