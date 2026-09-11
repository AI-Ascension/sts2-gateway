// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn unknown_recovery_receipts_use_the_route_generation_fence() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let rejoin_request = value(REJOIN_REQUEST);
    let rejoin_headers = headers(&rejoin_request);

    let mut rejoin_generation_drift = value(REJOIN_RESPONSE);
    rejoin_generation_drift["receipt"]["after_host_generation"] = 2.into();
    let bytes = serde_json::to_vec(&rejoin_generation_drift).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Rejoin,
                Some(&rejoin_request),
                &rejoin_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let mut unresolved_rejoin = value(REJOIN_RESPONSE);
    unresolved_rejoin["receipt"]["status"] = "unknown".into();
    unresolved_rejoin["receipt"]["after_host_generation"] = 1.into();
    let bytes = serde_json::to_vec(&unresolved_rejoin).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Rejoin,
                Some(&rejoin_request),
                &rejoin_headers,
                200,
                &bytes,
            )
            .is_err()
    );
}

#[test]
fn accepted_same_generation_is_reserved_for_pending_rejoin() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let recovered_request = value(RECOVER_REQUEST);
    let recovered_headers = headers(&recovered_request);
    let mut accepted_reconcile = value(REJOIN_RESPONSE);
    accepted_reconcile["operation_id"] = recovered_request["operation_id"].clone();
    accepted_reconcile["recovery"]["kind"] = "reconcile".into();
    accepted_reconcile["receipt"]["operation_id"] = recovered_request["operation_id"].clone();
    let bytes = serde_json::to_vec(&accepted_reconcile).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Recover,
                Some(&recovered_request),
                &recovered_headers,
                200,
                &bytes,
            )
            .is_err()
    );
}

#[test]
fn accepted_or_unknown_outcomes_cannot_cross_the_request_generation_fence() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);

    let action_request = value(UNKNOWN_REQUEST);
    let action_headers = headers(&action_request);
    let mut action_response = value(UNKNOWN_RESPONSE);
    action_response["observation"]["host_generation"] = 2.into();
    action_response["receipt"]["before_host_generation"] = 2.into();
    let bytes = serde_json::to_vec(&action_response).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LocalAction,
                Some(&action_request),
                &action_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let rejoin_request = value(REJOIN_REQUEST);
    let rejoin_headers = headers(&rejoin_request);
    let mut rejoin_response = value(REJOIN_RESPONSE);
    rejoin_response["observation"]["host_generation"] = 2.into();
    rejoin_response["receipt"]["before_host_generation"] = 2.into();
    rejoin_response["receipt"]["after_host_generation"] = 2.into();
    let bytes = serde_json::to_vec(&rejoin_response).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Rejoin,
                Some(&rejoin_request),
                &rejoin_headers,
                200,
                &bytes,
            )
            .is_err()
    );
}
