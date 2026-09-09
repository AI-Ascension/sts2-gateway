// SPDX-License-Identifier: MIT

use super::super::super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::super::{RuntimeV4ExpertRestActionForwarder, fixture};
use super::{headers, prepare, run_cycle};
use serde_json::Value;

#[test]
fn late_replays_cannot_remove_or_rewind_a_reused_selector() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    run_cycle(
        &mut forwarder,
        0,
        0,
        false,
        "rest-selection",
        "session:reused",
        4,
    )?;

    let old_first_request = prepare(
        fixture("first-request", 0)?,
        0,
        0,
        false,
        "rest-selection",
        "session:reused",
        4,
    )?;
    let mut old_progress = prepare(
        fixture("progressed", 0)?,
        0,
        0,
        false,
        "rest-selection",
        "session:reused",
        4,
    )?;
    old_progress["action"]["action_id"] = old_first_request["action"]["action_id"].clone();
    let old_confirm_request = prepare(
        fixture("confirm-request", 0)?,
        0,
        0,
        false,
        "rest-selection",
        "session:reused",
        4,
    )?;
    let mut old_completed = prepare(
        fixture("completed", 0)?,
        0,
        0,
        false,
        "rest-selection",
        "session:reused",
        4,
    )?;
    old_completed["action"]["action_id"] = old_confirm_request["action"]["action_id"].clone();
    old_completed["effect_witness"]["operation_id"] = old_completed["operation_id"].clone();
    old_completed["transition"]["effect_witness"]["operation_id"] =
        old_completed["operation_id"].clone();

    let open_request = prepare(
        fixture("option-request", 1)?,
        1,
        10,
        true,
        "opaque-producer",
        "session:reused",
        4,
    )?;
    let requested = prepare(
        fixture("requested", 1)?,
        1,
        10,
        true,
        "opaque-producer",
        "session:reused",
        4,
    )?;
    super::super::dispatch(&mut forwarder, &open_request, &requested)?;
    assert_eq!(
        forwarder
            .selector_admissions
            .get("selection:0:smith")
            .map(|admission| admission.generation),
        Some(20)
    );

    let old_completion_route = RuntimeV4ExpertRestActionRoute::Reconcile(
        old_completed["operation_id"]
            .as_str()
            .ok_or_else(|| String::from("old completion operation missing"))?
            .to_owned(),
    );
    let old_completed_bytes =
        serde_json::to_vec(&old_completed).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &old_completion_route,
            &Value::Null,
            &headers(&old_completed)?,
            200,
            &old_completed_bytes,
        )
        .map_err(|error| format!("old completion replay: {error:?}"))?;
    assert_eq!(
        forwarder
            .selector_admissions
            .get("selection:0:smith")
            .map(|admission| admission.generation),
        Some(20)
    );

    let first_request = prepare(
        fixture("first-request", 1)?,
        1,
        10,
        true,
        "opaque-producer",
        "session:reused",
        4,
    )?;
    let mut progressed = prepare(
        fixture("progressed", 1)?,
        1,
        10,
        true,
        "opaque-producer",
        "session:reused",
        4,
    )?;
    progressed["action"]["action_id"] = first_request["action"]["action_id"].clone();
    super::super::dispatch(&mut forwarder, &first_request, &progressed)?;
    assert_eq!(
        forwarder
            .selector_admissions
            .get("selection:0:smith")
            .map(|admission| admission.generation),
        Some(21)
    );

    let old_progress_route = RuntimeV4ExpertRestActionRoute::Reconcile(
        old_progress["operation_id"]
            .as_str()
            .ok_or_else(|| String::from("old progress operation missing"))?
            .to_owned(),
    );
    let old_progress_bytes =
        serde_json::to_vec(&old_progress).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &old_progress_route,
            &Value::Null,
            &headers(&old_progress)?,
            200,
            &old_progress_bytes,
        )
        .map_err(|error| format!("old progress replay: {error:?}"))?;
    assert_eq!(
        forwarder
            .selector_admissions
            .get("selection:0:smith")
            .map(|admission| admission.generation),
        Some(21)
    );
    Ok(())
}
