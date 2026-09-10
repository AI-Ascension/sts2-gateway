// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::Rc;

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2Authority, RuntimeV2CombatPhase, RuntimeV2EffectWitness,
    RuntimeV2ForwardRequest, RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2LedgerError, RuntimeV2Message, RuntimeV2MessageKind, RuntimeV2Metadata,
    RuntimeV2Observation, RuntimeV2ReceiptRequest, RuntimeV2ReceiptRetention,
    RuntimeV2RecoveryCapabilities, RuntimeV2RecoveryContract, RuntimeV2RecoveryError,
    RuntimeV2RecoveryFailureDomain, RuntimeV2Status, RuntimeV2TransportFault,
};

struct ForwardState {
    dispatches: usize,
    receipt_reads: usize,
    retained_receipt: Option<RuntimeV2Message>,
}

#[derive(Clone)]
struct CountingForwarder(Rc<RefCell<ForwardState>>);

impl CountingForwarder {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(ForwardState {
            dispatches: 0,
            receipt_reads: 0,
            retained_receipt: None,
        })))
    }

    fn dispatches(&self) -> usize {
        self.0.borrow().dispatches
    }

    fn receipt_reads(&self) -> usize {
        self.0.borrow().receipt_reads
    }
}

impl RuntimeV2ForwardingPort for CountingForwarder {
    fn forward_runtime_v2(
        &mut self,
        request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        let receipt = settled_response(request.message())?;
        let mut state = self.0.borrow_mut();
        state.dispatches += 1;
        state.retained_receipt = Some(receipt);
        Err(RuntimeV2TransportFault::DisconnectedAfterWrite)
    }

    fn read_runtime_v2_receipt(
        &mut self,
        request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        let mut state = self.0.borrow_mut();
        state.receipt_reads += 1;
        Ok(state.retained_receipt.clone().map(|mut receipt| {
            receipt.correlation_id = request.message().correlation_id.clone();
            receipt
        }))
    }
}

fn authority(boot_epoch: &str) -> Result<RuntimeV2Authority, String> {
    RuntimeV2Authority::new("instance-1", "session-1", "lease-1", 1, boot_epoch)
        .map_err(|error| error.to_string())
}

fn contract(
    boot_epoch: &str,
    receipt_retention: RuntimeV2ReceiptRetention,
) -> Result<RuntimeV2RecoveryContract, String> {
    RuntimeV2RecoveryContract::new(
        authority(boot_epoch)?,
        RuntimeV2RecoveryCapabilities::new(true, true, false, false, receipt_retention),
    )
    .map_err(|error| error.to_string())
}

fn observation() -> RuntimeV2Observation {
    RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4)
}

fn action_request(correlation_id: &str, operation_id: &str) -> RuntimeV2Message {
    RuntimeV2Message::action_request(
        RuntimeV2Metadata::new(),
        correlation_id,
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        operation_id,
        RuntimeV2Action::end_turn(),
    )
}

fn reconcile_request(correlation_id: &str, operation_id: &str) -> RuntimeV2Message {
    RuntimeV2Message::reconcile_request(
        RuntimeV2Metadata::new(),
        correlation_id,
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        operation_id,
    )
}

fn settled_response(
    request: &RuntimeV2Message,
) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
    let Some(operation_id) = request.operation_id.as_deref() else {
        return Err(RuntimeV2TransportFault::MalformedResponse);
    };
    let Some(action) = request.action.clone() else {
        return Err(RuntimeV2TransportFault::MalformedResponse);
    };
    Ok(RuntimeV2Message::result(
        RuntimeV2Metadata::new(),
        &request.correlation_id,
        &request.instance_id,
        &request.session_id,
        &request.lease_id,
        request.lease_epoch,
        5,
        operation_id,
        action,
        RuntimeV2Status::Settled,
        Some(RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            3,
            true,
            5,
        )),
        None,
        Some(RuntimeV2EffectWitness::turn_end_settled(5)),
        RuntimeV2MessageKind::ActionResponse,
    ))
}

fn workflow_ledger(
    retention: RuntimeV2ReceiptRetention,
    forwarder: CountingForwarder,
) -> Result<RuntimeV2Ledger<CountingForwarder>, String> {
    RuntimeV2Ledger::new_with_recovery_contract(
        RuntimeV2LedgerConfig::new(4),
        contract("boot-2", retention)?,
        observation(),
        forwarder,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn recovery_domains_and_receipt_scope_are_explicit() -> Result<(), String> {
    let capabilities = RuntimeV2RecoveryCapabilities::new(
        true,
        false,
        true,
        false,
        RuntimeV2ReceiptRetention::process_lifetime(8),
    );
    assert!(capabilities.supports(RuntimeV2RecoveryFailureDomain::HarnessRestart));
    assert!(!capabilities.supports(RuntimeV2RecoveryFailureDomain::GatewayRestart));
    assert!(capabilities.supports(RuntimeV2RecoveryFailureDomain::HostRestart));
    assert!(!capabilities.supports(RuntimeV2RecoveryFailureDomain::MachineReboot));
    assert_eq!(
        capabilities.require(RuntimeV2RecoveryFailureDomain::GatewayRestart),
        Err(RuntimeV2RecoveryError::UnsupportedFailureDomain(
            RuntimeV2RecoveryFailureDomain::GatewayRestart,
        ))
    );
    assert!(capabilities.receipt_retention().is_available());
    let unavailable = RuntimeV2RecoveryContract::new(
        authority("boot-2")?,
        RuntimeV2RecoveryCapabilities::unsupported(),
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(
        unavailable.require_receipt_access(),
        Err(RuntimeV2RecoveryError::ReceiptRetentionUnavailable)
    );
    assert_eq!(
        RuntimeV2RecoveryContract::new(
            authority("boot-2")?,
            RuntimeV2RecoveryCapabilities::new(
                false,
                false,
                false,
                false,
                RuntimeV2ReceiptRetention::durable(4, 0),
            ),
        ),
        Err(RuntimeV2RecoveryError::InvalidReceiptRetention)
    );
    Ok(())
}

#[test]
fn workflow_mutation_requires_current_authority_and_replays_without_dispatch() -> Result<(), String>
{
    let forwarder = CountingForwarder::new();
    let mut ledger = workflow_ledger(
        RuntimeV2ReceiptRetention::process_lifetime(8),
        forwarder.clone(),
    )?;
    let current = authority("boot-2")?;
    let stale = authority("boot-1")?;
    let request = action_request("corr-first", "operation-1");

    assert_eq!(
        ledger.submit_action(request.clone()),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::AuthorityRequired,
        ))
    );
    assert_eq!(
        ledger.submit_action_with_authority(&stale, request.clone()),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::StaleBootEpoch,
        ))
    );
    let unknown = ledger
        .submit_action_with_authority(&current, request.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(unknown.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(forwarder.dispatches(), 1);

    assert_eq!(
        ledger.submit_action_with_authority(&stale, request.clone()),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::StaleBootEpoch,
        ))
    );
    let replay = ledger
        .submit_action_with_authority(&current, request)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(forwarder.dispatches(), 1);
    Ok(())
}

#[test]
fn implicit_workflow_state_cancel_and_checkpoint_paths_require_authority() -> Result<(), String> {
    let forwarder = CountingForwarder::new();
    let mut ledger = workflow_ledger(
        RuntimeV2ReceiptRetention::process_lifetime(8),
        forwarder.clone(),
    )?;
    let action = action_request("corr-action", "operation-entrypoint");
    assert_eq!(
        ledger.cancel_before_dispatch(action.clone()),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::AuthorityRequired,
        ))
    );
    assert_eq!(
        ledger.submit_action_with_checkpoint(action, |_| Ok(())),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::AuthorityRequired,
        ))
    );

    let state_request = RuntimeV2Message::state_request(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
    );
    let state_response = RuntimeV2Message::state_response(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        observation(),
    );
    assert_eq!(
        ledger.accept_state_response(&state_request, state_response),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::AuthorityRequired,
        ))
    );
    assert_eq!(
        ledger.reconcile(reconcile_request("corr-reconcile", "operation-entrypoint")),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::AuthorityRequired,
        ))
    );
    assert_eq!(forwarder.dispatches(), 0);
    assert_eq!(forwarder.receipt_reads(), 0);
    Ok(())
}

#[test]
fn retained_receipt_recovery_is_capability_gated_and_read_only() -> Result<(), String> {
    let unavailable_forwarder = CountingForwarder::new();
    let mut unavailable = workflow_ledger(
        RuntimeV2ReceiptRetention::unavailable(),
        unavailable_forwarder.clone(),
    )?;
    let current = authority("boot-2")?;
    unavailable
        .submit_action_with_authority(&current, action_request("corr-action", "operation-2"))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        unavailable.reconcile_with_authority(
            &current,
            reconcile_request("corr-reconcile", "operation-2"),
        ),
        Err(RuntimeV2LedgerError::Recovery(
            RuntimeV2RecoveryError::ReceiptRetentionUnavailable,
        ))
    );
    assert_eq!(unavailable_forwarder.receipt_reads(), 0);

    let retained_forwarder = CountingForwarder::new();
    let mut retained = workflow_ledger(
        RuntimeV2ReceiptRetention::process_lifetime(8),
        retained_forwarder.clone(),
    )?;
    retained
        .submit_action_with_authority(&current, action_request("corr-action", "operation-3"))
        .map_err(|error| error.to_string())?;
    let settled = retained
        .reconcile_with_authority(&current, reconcile_request("corr-reconcile", "operation-3"))
        .map_err(|error| error.to_string())?;
    assert_eq!(settled.status, Some(RuntimeV2Status::Settled));
    assert_eq!(retained_forwarder.dispatches(), 1);
    assert_eq!(retained_forwarder.receipt_reads(), 1);
    Ok(())
}

#[test]
fn workflow_restore_rejects_missing_or_changed_boot_identity() -> Result<(), String> {
    let forwarder = CountingForwarder::new();
    let current = authority("boot-2")?;
    let mut ledger = workflow_ledger(
        RuntimeV2ReceiptRetention::process_lifetime(8),
        forwarder.clone(),
    )?;
    ledger
        .submit_action_with_authority(&current, action_request("corr-action", "operation-4"))
        .map_err(|error| error.to_string())?;
    let persisted = ledger.persisted_state();
    assert_eq!(persisted.boot_epoch.as_deref(), Some("boot-2"));

    let restarted_forwarder = CountingForwarder::new();
    let mut restarted = RuntimeV2Ledger::new_with_recovery_contract(
        RuntimeV2LedgerConfig::new(4),
        contract("boot-3", RuntimeV2ReceiptRetention::process_lifetime(8))?,
        observation(),
        restarted_forwarder,
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(
        restarted.restore_state(persisted.clone()),
        Err(RuntimeV2LedgerError::PersistedStateMismatch)
    );

    let mut missing_boot = persisted;
    missing_boot.boot_epoch = None;
    assert_eq!(
        restarted.restore_state(missing_boot),
        Err(RuntimeV2LedgerError::PersistedStateMismatch)
    );
    Ok(())
}
