// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::{SaveProfileContext, SaveProfileOperation};

use super::*;

#[test]
fn fixed_targets_do_not_accept_a_caller_path() {
    let (method, path) = route_target(SaveProfileRoute::Select, "op");
    assert_eq!(method, "POST");
    assert_eq!(path, "/api/v1/save-profile/select");
    assert!(!path.contains("op"));
}

#[test]
fn status_mapping_keeps_unknown_explicit() {
    assert_eq!(
        response_status(503, &json!({"status":"unknown"})),
        SaveProfileStatus::Unknown
    );
}

#[test]
fn lookup_receipt_keeps_the_original_operation_route() {
    let request = SaveProfileForwardRequest {
        operation_id: String::from("op-list"),
        context: SaveProfileContext {
            instance_id: String::from("instance-1"),
            caller_id: String::from("caller-1"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            correlation_id: String::from("corr-list"),
        },
        route: SaveProfileRoute::List,
        operation: SaveProfileOperation::List,
        body: Vec::new(),
    };
    let body = json!({
        "status": "settled",
        "instance_id": "instance-1",
        "caller_id": "caller-1",
        "session_id": "session-1",
        "lease_id": "lease-1",
        "lease_epoch": 1,
        "correlation_id": "corr-list",
        "operation_id": "op-list"
    });
    let decoded = decode_lookup_response(
        request,
        super::super::http::HttpResponse {
            status: 200,
            body: serde_json::to_vec(&body).expect("synthetic response serializes"),
        },
    )
    .expect("synthetic lookup response validates");
    assert_eq!(decoded.route, SaveProfileRoute::List);
}
