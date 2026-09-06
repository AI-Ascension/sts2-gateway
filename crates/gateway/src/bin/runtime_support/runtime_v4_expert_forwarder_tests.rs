// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert::RuntimeV4ExpertRoute;
use super::RuntimeV4ExpertForwarder;

#[test]
fn route_requires_an_empty_get_body_and_accepts_the_canonical_observation() {
    let forwarder = RuntimeV4ExpertForwarder::new(16 * 1024, 128 * 1024);
    assert!(
        forwarder
            .validate_request(RuntimeV4ExpertRoute::State, &[])
            .is_ok()
    );
    assert!(
        forwarder
            .validate_request(RuntimeV4ExpertRoute::State, b"{}")
            .is_err()
    );
    let golden = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ));
    assert!(
        forwarder
            .validate_response(RuntimeV4ExpertRoute::State, golden)
            .is_ok()
    );
}

#[test]
fn malformed_or_wrong_digest_observations_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV4ExpertForwarder::new(16 * 1024, 128 * 1024);
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))?;
    value["schema_digest"] = serde_json::Value::String(String::from("0").repeat(64));
    let bytes = serde_json::to_vec(&value)?;
    assert!(
        forwarder
            .validate_response(RuntimeV4ExpertRoute::State, &bytes)
            .is_err()
    );
    Ok(())
}
