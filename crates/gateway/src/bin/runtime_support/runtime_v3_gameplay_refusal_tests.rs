// SPDX-License-Identifier: MIT

//! The admitted shape of the refused-launch-contract recovery code (`sts2-gateway#85`).

use super::*;

#[test]
fn a_refused_launch_contract_code_is_admitted_only_in_the_shape_the_mod_composes()
-> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV3GameplayForwarder::new(16 * 1024, 128 * 1024);
    let mut request = fixture("state-request.json")?;
    request["kind"] = "legal_actions_request".into();
    request["state_id"] = "combat-1".into();
    let route = RuntimeV3GameplayRoute::LegalActions;
    let body = |code: &str| -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(
            &serde_json::json!({"correlation_id": request["correlation_id"],
            "error_code": code, "recovery": "reobserve"}),
        )
    };
    let admitted = |code: &str| {
        body(code)
            .is_ok_and(|bytes| forwarder.is_legal_actions_recovery(route, &request, 503, &bytes))
    };
    let longest = format!("launch_contract_refused_{}", "a".repeat(64));
    for code in [
        "launch_contract_refused",
        "launch_contract_refused_a",
        "launch_contract_refused_isolated_user_dir_mismatch",
        "launch_contract_refused_campaign_required",
        "launch_contract_refused_UPPER_lower-123_456",
        // `_` is a legal token character, so a token may begin with one; mirroring the
        // producer's rule admits it without enumerating today's reasons.
        "launch_contract_refused__leading",
        longest.as_str(),
    ] {
        assert!(admitted(code), "the refusal code {code} must be admitted");
    }
    // Each of these is a code the mod cannot compose, so admitting one would invent a second
    // vocabulary beside the producer's.
    for code in [
        "launch_contract_refused_",
        "launch_contract_refused_..",
        "launch_contract_refused_a b",
        "launch_contract_refused_a.b",
        "launch_contract_refused_a/b",
        "launch_contract_refused_ünicode",
        "launch_contract_refusedx",
        "launch_contractrefused",
        "launch_contract",
        "host_not_configured_refused",
    ] {
        assert!(!admitted(code), "{code} must stay refused");
    }
    // One byte over the reason bound is the first token the producer degrades to the bare prefix,
    // so the neighbouring admitted code is the 64-byte token and not this.
    assert!(!admitted(&format!(
        "launch_contract_refused_{}",
        "a".repeat(65)
    )));
    // Status, route, key set, duplicate keys and the size bound are already exercised for these
    // codes by the shared recovery table; only the refusal's own statuses and routes are new here.
    let refusal = body("launch_contract_refused_isolated_user_dir_mismatch")?;
    for status in [200, 409, 500, 502] {
        assert!(!forwarder.is_legal_actions_recovery(route, &request, status, &refusal));
    }
    for other in [
        RuntimeV3GameplayRoute::State,
        RuntimeV3GameplayRoute::DispatchAction,
        RuntimeV3GameplayRoute::WaitForTransition,
    ] {
        assert!(!forwarder.is_legal_actions_recovery(other, &request, 503, &refusal));
    }
    Ok(())
}
