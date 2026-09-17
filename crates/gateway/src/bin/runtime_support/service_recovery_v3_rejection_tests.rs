// SPDX-License-Identifier: MIT

use super::*;

/// A host refusal is a typed terminal `rejected` outcome. The canonical schema
/// requires a non-null `error_code` and a null `transition` for `rejected`, so a
/// settled-only translation silently degrades every refusal to `unknown` and
/// destroys the distinction between "the host refused this action" and "the
/// settlement could not be proven".
#[test]
fn rejected_translation_returns_a_typed_rejection_not_unknown() -> Result<(), String> {
    run_runtime_v3_translation_case_with_host_status(
        true,
        STATE,
        1,
        Duration::ZERO,
        false,
        "REJECTED",
    )
}

/// The typed rejection must not become a way to claim an observation that was
/// never read: with no authenticated post-settlement state the refusal still
/// fails closed as `unknown`.
#[test]
fn rejected_translation_still_fails_closed_without_an_observation() -> Result<(), String> {
    run_runtime_v3_translation_case_with_host_status(
        false,
        STATE,
        1,
        Duration::ZERO,
        false,
        "REJECTED",
    )
}
