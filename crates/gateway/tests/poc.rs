// SPDX-License-Identifier: MIT

#[allow(dead_code)]
mod support;

use sts2_gateway::{
    FenceFailure, FixedRoute, GatewayError, LeaseEpoch, LeaseProof, OperationId,
    verify_poc_artifact,
};
use support::{new_gateway, owner, ready_gateway, session};

#[test]
fn allocates_ready_instance_routes_fixed_body_and_fences_stale_or_wrong_leases()
-> Result<(), String> {
    verify_poc_artifact().map_err(|error| error.to_string())?;
    let (mut gateway, _clock, process, _readiness, transport) = new_gateway();
    let first = ready_gateway(&mut gateway)?;
    let second = ready_gateway(&mut gateway)?;

    let response = gateway
        .forward(
            first.instance_id(),
            first.lease().proof(),
            OperationId::new(1),
            FixedRoute::Command,
            vec![1, 2, 3],
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(response.status(), 200);
    assert_eq!(transport.calls(), 1);
    let request = transport
        .last_request()
        .ok_or_else(|| "successful request was not captured".to_owned())?;
    assert_eq!(request.instance_id(), first.instance_id());
    let expected_process = process
        .handle_for(first.instance_id())
        .ok_or_else(|| "first process handle was not allocated".to_owned())?;
    assert_eq!(request.process(), expected_process);
    assert_eq!(request.lease(), first.lease().proof());
    assert_eq!(request.operation_id(), OperationId::new(1));
    assert_eq!(request.route(), FixedRoute::Command);
    assert_eq!(request.body(), &[1, 2, 3]);
    assert_eq!(
        gateway
            .status(first.instance_id())
            .map_err(|error| error.to_string())?
            .state(),
        sts2_gateway::LifecycleState::Ready
    );

    let stale = LeaseProof::new(
        first.instance_id(),
        owner(),
        session(),
        first.lease().lease_id(),
        LeaseEpoch::new(0),
    );
    assert_eq!(
        gateway.forward(
            first.instance_id(),
            stale,
            OperationId::new(2),
            FixedRoute::Command,
            vec![4],
        ),
        Err(GatewayError::Fence(FenceFailure::StaleEpoch))
    );
    assert_eq!(transport.calls(), 1);
    assert_eq!(transport.last_request(), Some(request.clone()));
    assert_eq!(
        gateway.forward(
            second.instance_id(),
            first.lease().proof(),
            OperationId::new(3),
            FixedRoute::ReadOnly,
            vec![5],
        ),
        Err(GatewayError::Fence(FenceFailure::WrongInstance))
    );
    assert_eq!(transport.calls(), 1);
    assert_eq!(transport.last_request(), Some(request));
    Ok(())
}
