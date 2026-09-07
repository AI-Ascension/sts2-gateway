// SPDX-License-Identifier: MIT

fn operation_ref_matches(value: &Value, operation: &RecoveryOperation) -> bool {
    value["operation_id"].as_str() == Some(operation.operation_id.as_str())
        && value["payload_digest"].as_str() == Some(operation.payload_digest.as_str())
        && value["original_context"]["deployment_id"].as_str()
            == Some(operation.deployment_id.as_str())
        && value["original_context"]["instance_id"].as_str() == Some(operation.instance_id.as_str())
        && value["original_context"]["instance_incarnation"].as_str()
            == Some(operation.instance_incarnation.as_str())
        && value["original_context"]["boot_id"].as_str() == Some(operation.boot_id.as_str())
        && value["original_context"]["authority_generation"].as_u64()
            == Some(operation.authority_generation)
        && value["original_context"]["lease_id"].as_str() == Some(operation.lease_id.as_str())
        && value["original_context"]["lease_epoch"].as_u64() == Some(operation.lease_epoch)
}

fn validate_host_lookup_response(
    response: &Value,
    operation: &RecoveryOperation,
) -> Result<(), &'static str> {
    let payload = super::recovery_wire::exact_object(
        &response["payload"],
        &["result", "operation", "mutation_authorized"],
    )
    .ok_or("host_lookup_payload_invalid")?;
    if payload["mutation_authorized"] != Value::Bool(false) {
        return Err("host_lookup_mutation_authorized");
    }
    let status = validate_host_result(&payload["result"])?;
    if status == "NOT_FOUND" {
        if !payload["operation"].is_null() {
            return Err("host_lookup_not_found_operation");
        }
        return Ok(());
    }
    if !matches!(status, "ACCEPTED" | "SETTLED" | "REJECTED" | "UNKNOWN") {
        return Err("host_lookup_result_invalid");
    }
    validate_host_operation(&payload["operation"], operation, false)
}

fn validate_host_reconcile_response(
    response: &Value,
    operation: &RecoveryOperation,
) -> Result<(), &'static str> {
    let payload = super::recovery_wire::exact_object(
        &response["payload"],
        &["result", "operation", "witness"],
    )
    .ok_or("host_reconcile_payload_invalid")?;
    let status = validate_host_result(&payload["result"])?;
    if status == "NOT_FOUND" {
        if !payload["operation"].is_null() || !payload["witness"].is_null() {
            return Err("host_reconcile_not_found_operation");
        }
        return Ok(());
    }
    if !matches!(status, "ACCEPTED" | "SETTLED" | "REJECTED" | "UNKNOWN") {
        return Err("host_reconcile_result_invalid");
    }
    validate_host_operation(&payload["operation"], operation, true)?;
    if !payload["witness"].is_null() && !payload["operation"]["witness"].is_null() {
        if payload["witness"] != payload["operation"]["witness"] {
            return Err("host_reconcile_witness_mismatch");
        }
    } else if !payload["witness"].is_null() || !payload["operation"]["witness"].is_null() {
        return Err("host_reconcile_witness_missing");
    }
    Ok(())
}

fn validate_host_result(value: &Value) -> Result<&str, &'static str> {
    let object =
        super::recovery_wire::exact_object(value, &["status", "retryable", "retry_after_seconds"])
            .ok_or("host_result_invalid")?;
    let status = object["status"]
        .as_str()
        .ok_or("host_result_status_invalid")?;
    if object["retryable"].as_bool().is_none()
        || (!object["retry_after_seconds"].is_null()
            && object["retry_after_seconds"].as_u64().is_none())
    {
        return Err("host_result_retry_invalid");
    }
    if object["retryable"].as_bool() == Some(true)
        && object["retry_after_seconds"].as_u64() != Some(1)
    {
        return Err("host_result_retry_invalid");
    }
    Ok(status)
}

fn validate_host_operation(
    value: &Value,
    operation: &RecoveryOperation,
    reconcile: bool,
) -> Result<(), &'static str> {
    let object = super::recovery_wire::exact_object(
        value,
        &[
            "operation_id",
            "state",
            "payload_digest",
            "original_context",
            "expected_boundary",
            "action",
            "ticket",
            "witness",
            "uncertainty_reason",
            "created_at",
            "updated_at",
        ],
    )
    .ok_or("host_operation_shape_invalid")?;
    if !super::recovery_dispatch_host::host_operation_identity_matches(value, operation) {
        return Err("host_operation_identity_mismatch");
    }
    let state = parse_operation_state(
        object["state"]
            .as_str()
            .ok_or("host_operation_state_invalid")?,
    )
    .ok_or("host_operation_state_invalid")?;
    let created_at = super::recovery_dispatch_host::parse_timestamp_millis(
        object["created_at"]
            .as_str()
            .ok_or("host_operation_time_invalid")?,
    )
    .ok_or("host_operation_time_invalid")?;
    let updated_at = super::recovery_dispatch_host::parse_timestamp_millis(
        object["updated_at"]
            .as_str()
            .ok_or("host_operation_time_invalid")?,
    )
    .ok_or("host_operation_time_invalid")?;
    if !validate_expected_boundary(&object["expected_boundary"], operation)
        || !validate_action(&object["action"], operation)
        || updated_at < created_at
    {
        return Err("host_operation_context_invalid");
    }
    if let Some(reason) = object["uncertainty_reason"].as_str() {
        if !matches!(
            reason,
            "transport_lost"
                | "timeout"
                | "gateway_crash"
                | "host_crash"
                | "receipt_missing"
                | "authority_rotated"
        ) {
            return Err("host_operation_uncertainty_invalid");
        }
    } else if !object["uncertainty_reason"].is_null() {
        return Err("host_operation_uncertainty_invalid");
    }
    let ticket = if object["ticket"].is_null() {
        None
    } else {
        super::recovery_dispatch_host::parse_historical_host_ticket(
            Some(&object["ticket"]),
            operation,
        )
    };
    if !object["ticket"].is_null() && ticket.is_none() {
        return Err("host_operation_ticket_invalid");
    }
    let witness = if object["witness"].is_null() {
        None
    } else {
        super::recovery_dispatch_host::parse_historical_host_witness(
            Some(&object["witness"]),
            operation,
            ticket.as_ref().map(|ticket| ticket.host_fence_id.as_str()),
        )
    };
    if !object["witness"].is_null() && witness.is_none() {
        return Err("host_operation_witness_invalid");
    }
    if witness.is_some() && ticket.is_none() {
        return Err("host_operation_witness_ticket_missing");
    }
    match state {
        RecoveryOperationState::Settled | RecoveryOperationState::Reconciled
            if witness.is_none() || ticket.is_none() =>
        {
            return Err("host_operation_settlement_evidence_missing");
        }
        RecoveryOperationState::Accepted if witness.is_some() => {
            return Err("host_operation_accepted_witness_invalid");
        }
        _ => {}
    }
    if reconcile && state == RecoveryOperationState::Reconciled && witness.is_none() {
        return Err("host_operation_reconciled_witness_missing");
    }
    Ok(())
}

fn parse_host_evidence(
    response: &Value,
    operation: &RecoveryOperation,
    reconcile: bool,
) -> Result<HostOperationEvidence, &'static str> {
    let payload = &response["payload"];
    let result_status = payload["result"]["status"]
        .as_str()
        .ok_or("host_result_status_invalid")?
        .to_owned();
    let object = &payload["operation"];
    let operation_state = parse_operation_state(
        object["state"]
            .as_str()
            .ok_or("host_operation_state_invalid")?,
    )
    .ok_or("host_operation_state_invalid")?;
    let ticket = if object["ticket"].is_null() {
        None
    } else {
        Some(
            super::recovery_dispatch_host::parse_historical_host_ticket(
                Some(&object["ticket"]),
                operation,
            )
            .ok_or("host_operation_ticket_invalid")?,
        )
    };
    let witness = if payload["witness"].is_null() {
        if object["witness"].is_null() {
            None
        } else {
            Some(
                super::recovery_dispatch_host::parse_historical_host_witness(
                    Some(&object["witness"]),
                    operation,
                    ticket.as_ref().map(|ticket| ticket.host_fence_id.as_str()),
                )
                .ok_or("host_operation_witness_invalid")?,
            )
        }
    } else {
        Some(
            super::recovery_dispatch_host::parse_historical_host_witness(
                Some(&payload["witness"]),
                operation,
                ticket.as_ref().map(|ticket| ticket.host_fence_id.as_str()),
            )
            .ok_or("host_reconcile_witness_invalid")?,
        )
    };
    let status_matches_state = match result_status.as_str() {
        "ACCEPTED" => operation_state == RecoveryOperationState::Accepted,
        "SETTLED" => matches!(
            operation_state,
            RecoveryOperationState::Settled | RecoveryOperationState::Reconciled
        ),
        "REJECTED" => operation_state == RecoveryOperationState::Rejected,
        "UNKNOWN" => matches!(
            operation_state,
            RecoveryOperationState::IntentRecorded
                | RecoveryOperationState::MayHaveBeenDispatched
                | RecoveryOperationState::Unknown
                | RecoveryOperationState::Reconciled
        ),
        _ => false,
    };
    if !status_matches_state {
        return Err("host_operation_status_state_mismatch");
    }
    if reconcile && witness.is_some() && operation_state == RecoveryOperationState::IntentRecorded {
        return Err("host_reconcile_witness_state_invalid");
    }
    Ok(HostOperationEvidence {
        result_status,
        operation_state,
        ticket,
        witness,
    })
}
