// SPDX-License-Identifier: MIT

use sts2_gateway::{RuntimeV2Message, runtime_v2_artifact_files};

#[test]
fn every_golden_requires_each_nullable_member() -> Result<(), Box<dyn std::error::Error>> {
    for golden in runtime_v2_artifact_files().goldens {
        let value: serde_json::Value = serde_json::from_slice(golden.bytes)?;
        let message: RuntimeV2Message = serde_json::from_value(value.clone())?;
        message.validate()?;
        assert_eq!(serde_json::to_value(message)?, value, "{}", golden.path);
        for field in [
            "operation_id",
            "observation",
            "action",
            "status",
            "error_code",
            "effect_witness",
        ] {
            let mut incomplete = value.clone();
            let object = incomplete
                .as_object_mut()
                .ok_or("golden is not an object")?;
            assert!(object.remove(field).is_some(), "{}: {field}", golden.path);
            assert!(
                serde_json::from_value::<RuntimeV2Message>(incomplete).is_err(),
                "{} accepted missing {field}",
                golden.path
            );
        }
    }
    Ok(())
}

#[test]
fn every_golden_rejects_unknown_envelope_members() -> Result<(), Box<dyn std::error::Error>> {
    for golden in runtime_v2_artifact_files().goldens {
        let mut value: serde_json::Value = serde_json::from_slice(golden.bytes)?;
        value
            .as_object_mut()
            .ok_or("golden is not an object")?
            .insert(String::from("unexpected"), serde_json::Value::Null);
        assert!(
            serde_json::from_value::<RuntimeV2Message>(value).is_err(),
            "{}",
            golden.path
        );
    }
    Ok(())
}
