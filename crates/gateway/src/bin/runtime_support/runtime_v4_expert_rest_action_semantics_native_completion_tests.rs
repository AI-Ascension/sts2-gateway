// SPDX-License-Identifier: MIT

use super::completed_effect_valid;
use serde_json::Value;

macro_rules! golden {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/",
            $name
        )) as &'static [u8]
    };
}

fn golden_value(name: &str) -> Result<Value, String> {
    let bytes = match name {
        "mend" => golden!("action-mend-selection-completed.json"),
        "rest" => golden!("action-completed.json"),
        _ => return Err(format!("unknown golden {name}")),
    };
    serde_json::from_slice(bytes).map_err(|error| error.to_string())
}

fn native_evidence(value: &Value) -> Value {
    serde_json::json!({
        "kind": "native_completion",
        "completion_id": "completion:rest:1",
        "native_state_id": value["observation"]["state_id"],
    })
}

fn replace_effect_evidence(value: &mut Value, evidence: Value) {
    value["effect_witness"]["evidence"] = evidence;
    value["transition"]["effect_witness"] = value["effect_witness"].clone();
}

fn valid_native_completion(option: &str) -> Result<(Value, Value), String> {
    let mut value = golden_value(if option == "mend" { "mend" } else { "rest" })?;
    value["action"]["action"]["rest_option_id"] = option.into();
    value["transition"]["rest_option_id"] = option.into();
    value["effect_witness"]["rest_option_id"] = option.into();
    value["effect_witness"]["kind"] = format!("{option}_applied").into();
    let evidence = native_evidence(&value);
    replace_effect_evidence(&mut value, evidence);
    let transition = value["transition"].clone();
    Ok((value, transition))
}

#[test]
fn typed_native_completion_is_effectful_for_mend_lift_and_kindle() -> Result<(), String> {
    for option in ["mend", "lift", "kindle"] {
        let (value, transition) = valid_native_completion(option)?;
        assert!(completed_effect_valid(&value, &transition), "{option}");
    }
    Ok(())
}

#[test]
fn native_completion_requires_matching_state_and_completion_identity() -> Result<(), String> {
    for option in ["mend", "lift", "kindle"] {
        let (valid, transition) = valid_native_completion(option)?;
        assert!(
            completed_effect_valid(&valid, &transition),
            "valid {option}"
        );

        let mut stale = valid.clone();
        stale["effect_witness"]["evidence"]["native_state_id"] = "live:stale".into();
        stale["transition"]["effect_witness"] = stale["effect_witness"].clone();
        let stale_transition = stale["transition"].clone();
        assert!(
            !completed_effect_valid(&stale, &stale_transition),
            "stale {option}"
        );

        for field in ["completion_id", "native_state_id"] {
            let mut missing = valid.clone();
            let evidence = missing["effect_witness"]["evidence"]
                .as_object_mut()
                .ok_or_else(|| String::from("native evidence object missing"))?;
            evidence.remove(field);
            missing["transition"]["effect_witness"] = missing["effect_witness"].clone();
            let missing_transition = missing["transition"].clone();
            assert!(
                !completed_effect_valid(&missing, &missing_transition),
                "missing {field} {option}"
            );
        }
    }
    Ok(())
}
