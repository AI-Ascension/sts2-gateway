// SPDX-License-Identifier: MIT

use sts2_gateway::{
    SeededRunBinding, SeededRunForwardRequest, SeededRunForwardingPort, SeededRunLedger,
    SeededRunLedgerConfig, SeededRunLedgerError, SeededRunMessage, SeededRunReceiptRequest,
    SeededRunStatus, SeededRunTransportFault,
};

const START_REQUEST: &str =
    include_str!("../../../protocol-artifact/seeded-run-v1/golden/start-request.json");
const START_SETTLED: &str =
    include_str!("../../../protocol-artifact/seeded-run-v1/golden/start-settled.json");

#[derive(Default)]
struct FakeForwarder {
    starts: usize,
    receipts: usize,
    timeout_once: bool,
    accepted_once: bool,
    settled: Option<SeededRunMessage>,
    last_start: Option<SeededRunMessage>,
}

impl SeededRunForwardingPort for FakeForwarder {
    fn forward_seeded_run(
        &mut self,
        request: SeededRunForwardRequest,
    ) -> Result<SeededRunMessage, SeededRunTransportFault> {
        self.starts += 1;
        self.last_start = Some(request.message.clone());
        if self.timeout_once {
            self.timeout_once = false;
            return Err(SeededRunTransportFault::Timeout);
        }
        let mut response = self.settled_for(&request.message)?;
        if self.accepted_once {
            self.accepted_once = false;
            response.status = Some(SeededRunStatus::Accepted);
            response.canonical_seed = None;
            response.observation = None;
            response.effect_witness = None;
            response.error_code = None;
        }
        Ok(response)
    }

    fn read_seeded_run_receipt(
        &mut self,
        request: SeededRunReceiptRequest,
    ) -> Result<Option<SeededRunMessage>, SeededRunTransportFault> {
        self.receipts += 1;
        let original = self
            .last_start
            .as_ref()
            .ok_or(SeededRunTransportFault::MalformedResponse)?;
        let mut receipt = self.settled_for(original)?;
        receipt.kind = sts2_gateway::SeededRunMessageKind::ReconcileResponse;
        receipt.correlation_id = request.message.correlation_id;
        receipt.operation_id = request.operation_id;
        Ok(Some(receipt))
    }
}

impl FakeForwarder {
    fn settled_for(
        &self,
        request: &SeededRunMessage,
    ) -> Result<SeededRunMessage, SeededRunTransportFault> {
        let mut response = match self.settled.clone() {
            Some(response) => response,
            None => serde_json::from_str(START_SETTLED)
                .map_err(|_| SeededRunTransportFault::MalformedResponse)?,
        };
        response.correlation_id = request.correlation_id.clone();
        response.instance_id = request.instance_id.clone();
        response.session_id = request.session_id.clone();
        response.lease_id = request.lease_id.clone();
        response.lease_epoch = request.lease_epoch;
        response.generation = request.generation;
        response.operation_id = request.operation_id.clone();
        response.requested_seed = request.requested_seed.clone();
        response.run_mode = request.run_mode;
        response.context_digest = request.context_digest.clone();
        response.selected_context = request.selected_context.clone();
        if let Some(observation) = response.observation.as_mut() {
            observation.selected_context_digest = request.context_digest.clone();
            observation.generation = request.generation + 1;
        }
        if let Some(witness) = response.effect_witness.as_mut() {
            witness.generation = request.generation + 1;
        }
        Ok(response)
    }
}

fn request(operation_id: &str, correlation_id: &str) -> Result<SeededRunMessage, String> {
    let mut request: SeededRunMessage =
        serde_json::from_str(START_REQUEST).map_err(|error| error.to_string())?;
    request.operation_id = operation_id.to_owned();
    request.correlation_id = correlation_id.to_owned();
    Ok(request)
}

fn ledger(forwarder: FakeForwarder) -> Result<SeededRunLedger<FakeForwarder>, String> {
    SeededRunLedger::new(
        SeededRunLedgerConfig::new(8),
        SeededRunBinding::new("instance-1", "session-1", "lease-1", 1, 0)
            .map_err(|error| error.to_string())?,
        forwarder,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn duplicate_replays_without_second_host_start_and_conflicts_are_rejected() -> Result<(), String> {
    let mut ledger = ledger(FakeForwarder::default())?;
    let first = request("operation-1", "corr-1")?;
    let settled = ledger
        .submit_start(first.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(settled.status, Some(SeededRunStatus::Settled));
    assert_eq!(ledger.forwarding_mut().starts, 1);
    let mut duplicate = first.clone();
    duplicate.correlation_id = String::from("corr-2");
    let replay = ledger
        .submit_start(duplicate)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.status, Some(SeededRunStatus::Settled));
    assert_eq!(replay.correlation_id, "corr-2");
    assert!(replay.validate().is_ok());
    assert_eq!(ledger.forwarding_mut().starts, 1);

    let mut conflict = request("operation-1", "corr-3")?;
    conflict.requested_seed = Some(String::from("different-seed"));
    assert_eq!(
        ledger.submit_start(conflict),
        Err(SeededRunLedgerError::OperationConflict)
    );
    Ok(())
}

#[test]
fn accepted_start_reconciles_to_settled_without_replay() -> Result<(), String> {
    let mut ledger = ledger(FakeForwarder {
        accepted_once: true,
        ..FakeForwarder::default()
    })?;
    let start = request("operation-accepted", "corr-start")?;
    let accepted = ledger
        .submit_start(start)
        .map_err(|error| error.to_string())?;
    assert_eq!(accepted.status, Some(SeededRunStatus::Accepted));
    assert!(accepted.validate().is_ok());

    let reconcile = SeededRunMessage::reconcile_request(
        sts2_gateway::SeededRunProvenance::default(),
        sts2_gateway::SeededRunContext::new(
            "corr-reconcile",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            0,
        ),
        "operation-accepted",
    );
    let settled = ledger
        .reconcile(reconcile)
        .map_err(|error| error.to_string())?;
    assert_eq!(settled.status, Some(SeededRunStatus::Settled));
    assert_eq!(
        settled.kind,
        sts2_gateway::SeededRunMessageKind::ReconcileResponse
    );
    assert!(settled.validate().is_ok());
    assert_eq!(ledger.forwarding_mut().starts, 1);
    assert_eq!(ledger.forwarding_mut().receipts, 1);
    Ok(())
}

#[test]
fn unknown_reconciles_by_receipt_without_second_start() -> Result<(), String> {
    let mut ledger = ledger(FakeForwarder {
        timeout_once: true,
        ..FakeForwarder::default()
    })?;
    let start = request("operation-unknown", "corr-start")?;
    let unknown = ledger
        .submit_start(start)
        .map_err(|error| error.to_string())?;
    assert_eq!(unknown.status, Some(SeededRunStatus::Unknown));
    assert_eq!(ledger.forwarding_mut().starts, 1);

    let reconcile = SeededRunMessage::reconcile_request(
        sts2_gateway::SeededRunProvenance::default(),
        sts2_gateway::SeededRunContext::new(
            "corr-reconcile",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            0,
        ),
        "operation-unknown",
    );
    let settled = ledger
        .reconcile(reconcile)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        settled.kind,
        sts2_gateway::SeededRunMessageKind::ReconcileResponse
    );
    assert_eq!(settled.status, Some(SeededRunStatus::Settled));
    assert_eq!(ledger.forwarding_mut().starts, 1);
    assert_eq!(ledger.forwarding_mut().receipts, 1);
    Ok(())
}

#[test]
fn persisted_unknown_marker_survives_restart_without_replay() -> Result<(), String> {
    let mut first = ledger(FakeForwarder {
        timeout_once: true,
        ..FakeForwarder::default()
    })?;
    let start = request("operation-restart", "corr-start")?;
    let _ = first
        .submit_start(start.clone())
        .map_err(|error| error.to_string())?;
    let state = first.persisted_state();

    let mut restarted = ledger(FakeForwarder {
        last_start: Some(start),
        ..FakeForwarder::default()
    })?;
    restarted
        .restore_state(state)
        .map_err(|error| error.to_string())?;
    let reconcile = SeededRunMessage::reconcile_request(
        sts2_gateway::SeededRunProvenance::default(),
        sts2_gateway::SeededRunContext::new(
            "corr-reconcile",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            0,
        ),
        "operation-restart",
    );
    let result = restarted
        .reconcile(reconcile)
        .map_err(|error| error.to_string())?;
    assert_eq!(result.status, Some(SeededRunStatus::Settled));
    assert_eq!(restarted.forwarding_mut().starts, 0);
    assert_eq!(restarted.forwarding_mut().receipts, 1);
    Ok(())
}

#[test]
fn stale_lease_is_rejected_before_replay() -> Result<(), String> {
    let mut ledger = ledger(FakeForwarder::default())?;
    let start = request("operation-fence", "corr-fence")?;
    let _ = ledger
        .submit_start(start.clone())
        .map_err(|error| error.to_string())?;
    let mut stale = start;
    stale.lease_epoch = 2;
    assert_eq!(
        ledger.submit_start(stale),
        Err(SeededRunLedgerError::FenceRejected)
    );
    Ok(())
}

#[test]
fn copied_goldens_are_valid_contract_messages() -> Result<(), String> {
    let request: SeededRunMessage =
        serde_json::from_str(START_REQUEST).map_err(|error| error.to_string())?;
    let settled: SeededRunMessage =
        serde_json::from_str(START_SETTLED).map_err(|error| error.to_string())?;
    assert!(request.validate().is_ok());
    assert!(settled.validate().is_ok());
    Ok(())
}
