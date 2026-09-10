// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::RuntimeV4ExpertRestActionForwarder;
use serde_json::Value;
use std::collections::BTreeMap;

macro_rules! golden {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/",
            $name
        )) as &'static [u8]
    };
}

fn headers(value: &Value) -> Result<BTreeMap<String, String>, String> {
    let mut output = BTreeMap::new();
    for (field, header) in [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ] {
        output.insert(
            String::from(header),
            value[field]
                .as_str()
                .ok_or_else(|| format!("{field} missing"))?
                .to_owned(),
        );
    }
    output.insert(
        String::from("x-sts2-lease-epoch"),
        value["lease_epoch"]
            .as_u64()
            .ok_or_else(|| String::from("lease_epoch missing"))?
            .to_string(),
    );
    Ok(output)
}

fn status(value: &Value) -> Option<u16> {
    match value["status"].as_str()? {
        "accepted" => Some(202),
        "settled" => Some(200),
        "rejected" => Some(409),
        "unknown" => Some(404),
        "cancelled" => Some(499),
        _ => None,
    }
}

#[test]
fn candidate_mutations_are_rejected_by_the_gateway_consumer() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let mut requests = BTreeMap::new();
    for name in [
        "action-request.json",
        "action-selection-confirm-request.json",
        "action-selection-first-request.json",
        "action-selection-option-request.json",
        "action-selection-second-request.json",
    ] {
        let bytes = match name {
            "action-request.json" => golden!("action-request.json"),
            "action-selection-confirm-request.json" => {
                golden!("action-selection-confirm-request.json")
            }
            "action-selection-first-request.json" => golden!("action-selection-first-request.json"),
            "action-selection-option-request.json" => {
                golden!("action-selection-option-request.json")
            }
            "action-selection-second-request.json" => {
                golden!("action-selection-second-request.json")
            }
            _ => return Err(format!("unknown request golden: {name}")),
        };
        let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let operation = value["operation_id"]
            .as_str()
            .ok_or_else(|| format!("{name}: operation missing"))?;
        requests.insert(operation.to_owned(), (value.clone(), headers(&value)?));
    }
    let null_request = Value::Null;
    for name in [
        "action-accepted-empty-action.json",
        "action-accepted-unknown-action-kind.json",
        "action-accepted-unrelated-option-action.json",
        "action-completed-evidence-hp-change.json",
        "action-completed-evidence-native-completion.json",
        "action-completed-heal-noop.json",
        "action-completed-unrelated-option-action.json",
        "action-completed-untested-option-witness.json",
        "action-mend-selection-completed-absent-player.json",
        "action-mend-selection-completed-evidence-hp-change.json",
        "action-mend-selection-completed-evidence-native-completion.json",
        "action-selection-completed-bogus.json",
        "action-selection-completed-duplicates.json",
        "action-selection-completed-required-count-1.json",
        "action-selection-completed-selection-kind-player.json",
        "action-selection-completed-unrelated-selection-action.json",
        "action-selection-progressed-remaining-count-99.json",
        "action-selection-progressed-unlisted-choice.json",
        "action-selection-progressed-unrelated-selection-action.json",
        "action-selection-requested-no-choice-action.json",
        "action-selection-requested-option-kind-mismatch.json",
        "action-selection-requested-unrelated-selection-action.json",
    ] {
        let path = format!(
            "{}/../../conformance/mutations/runtime-v4-expert-rest-action-v1/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = std::fs::read(&path).map_err(|error| format!("{name}: {error}"))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| format!("{name}: {error}"))?;
        let operation = value["operation_id"]
            .as_str()
            .ok_or_else(|| format!("{name}: operation missing"))?;
        let response_status = status(&value).ok_or_else(|| format!("{name}: status missing"))?;
        let response_headers = headers(&value)?;
        let (route, request, request_headers) = requests
            .get(operation)
            .map(|(request, headers)| (RuntimeV4ExpertRestActionRoute::Dispatch, request, headers))
            .unwrap_or_else(|| {
                (
                    RuntimeV4ExpertRestActionRoute::Reconcile(operation.to_owned()),
                    &null_request,
                    &response_headers,
                )
            });
        assert!(
            forwarder
                .validate_response(&route, request, request_headers, response_status, &bytes,)
                .is_err(),
            "mutation unexpectedly validated: {name}"
        );
    }
    Ok(())
}
