// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::RuntimeV4ExpertRestActionForwarder;
use serde_json::Value;
use std::collections::BTreeMap;

macro_rules! producer_fixture {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/producer/",
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
fn producer_lifecycle_fixtures_validate_each_wire_message() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    for (fixture_name, fixture_bytes) in [
        (
            "producer/mend-selection-lifecycle.json",
            producer_fixture!("mend-selection-lifecycle.json"),
        ),
        (
            "producer/smith-selection-lifecycle.json",
            producer_fixture!("smith-selection-lifecycle.json"),
        ),
    ] {
        let fixture: Value = serde_json::from_slice(fixture_bytes)
            .map_err(|error| format!("{fixture_name}: {error}"))?;
        if fixture["fixture_version"] != "runtime-v4-expert-rest-action-producer-fixture-v1"
            || fixture["protocol_version"] != "runtime-v4-expert-rest-action-v1"
            || fixture["profile"] != "expert-rest-action"
            || fixture["schema_digest"]
                != "bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd"
        {
            return Err(format!("{fixture_name}: fixture metadata changed"));
        }
        let messages = fixture["messages"]
            .as_array()
            .ok_or_else(|| format!("{fixture_name}: messages missing"))?;
        if messages.is_empty() {
            return Err(format!("{fixture_name}: messages empty"));
        }
        for (index, message) in messages.iter().enumerate() {
            let label = format!("{fixture_name} message {index}");
            let request = &message["request"];
            let request_bytes = serde_json::to_vec(request)
                .map_err(|error| format!("{label} request encode: {error}"))?;
            let request_headers = headers(request).map_err(|error| format!("{label}: {error}"))?;
            let parsed_request = forwarder
                .validate_request(
                    &RuntimeV4ExpertRestActionRoute::Dispatch,
                    &request_bytes,
                    &request_headers,
                )
                .map_err(|error| format!("{label} request: {error:?}"))?;
            if parsed_request != *request {
                return Err(format!("{label}: request changed during validation"));
            }

            let response = &message["response"];
            let response_code =
                status(response).ok_or_else(|| format!("{label}: status missing"))?;
            let response_bytes = serde_json::to_vec(response)
                .map_err(|error| format!("{label} response encode: {error}"))?;
            // Producer fixtures model independent wire messages. For a real
            // dispatch, the response correlation is required to echo the
            // request; reconcile still checks the fixture's raw response.
            let mut dispatch_response = response.clone();
            dispatch_response["correlation_id"] = request["correlation_id"].clone();
            let dispatch_bytes = serde_json::to_vec(&dispatch_response)
                .map_err(|error| format!("{label} dispatch encode: {error}"))?;
            forwarder
                .validate_response(
                    &RuntimeV4ExpertRestActionRoute::Dispatch,
                    &parsed_request,
                    &request_headers,
                    response_code,
                    &dispatch_bytes,
                )
                .map_err(|error| format!("{label} dispatch response: {error:?}"))?;

            let operation = response["operation_id"]
                .as_str()
                .ok_or_else(|| format!("{label}: operation missing"))?;
            let response_headers =
                headers(response).map_err(|error| format!("{label}: {error}"))?;
            forwarder
                .validate_response(
                    &RuntimeV4ExpertRestActionRoute::Reconcile(operation.to_owned()),
                    &Value::Null,
                    &response_headers,
                    response_code,
                    &response_bytes,
                )
                .map_err(|error| format!("{label} reconciliation response: {error:?}"))?;
        }
    }
    Ok(())
}
