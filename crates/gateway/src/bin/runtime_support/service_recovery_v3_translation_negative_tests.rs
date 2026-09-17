// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn settled_translation_rejects_a_poststate_witness_mismatch() -> Result<(), String> {
    run_runtime_v3_translation_case(
        true,
        STATE,
        3,
        Duration::ZERO,
        false,
    )
}

#[test]
fn settled_translation_rejects_lease_expiry_during_blocked_observation() -> Result<(), String> {
    run_runtime_v3_translation_case(true, STATE, 2, Duration::from_millis(200), true)
}

pub(super) fn expected_error(state_ok: bool, witness_matches: bool) -> &'static str {
    if state_ok && !witness_matches {
        "settlement_witness_mismatch"
    } else {
        "settlement_observation_unavailable"
    }
}
