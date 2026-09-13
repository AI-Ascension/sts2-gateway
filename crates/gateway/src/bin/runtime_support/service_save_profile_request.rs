// SPDX-License-Identifier: MIT

use super::super::save_profile::RuntimeSaveProfileRoute;
use super::*;
use serde_json::Value;
use sts2_gateway::{
    LAUNCH_PROFILE_ID, LaunchProfileBinding, SaveProfileOperation, UserDataDescriptor,
    UserDataIdentity, UserDataProvenance,
};

pub(super) fn build_operation(
    route: RuntimeSaveProfileRoute,
    request: &HttpRequest,
    operation_id: &str,
) -> Result<(SaveProfileOperation, Vec<u8>), &'static str> {
    match route {
        RuntimeSaveProfileRoute::List | RuntimeSaveProfileRoute::Current => {
            if !request.body.is_empty() {
                return Err("save_profile_body_not_allowed");
            }
            let operation = if route == RuntimeSaveProfileRoute::List {
                SaveProfileOperation::List
            } else {
                SaveProfileOperation::Current
            };
            Ok((operation, Vec::new()))
        }
        RuntimeSaveProfileRoute::Select => select_operation(request),
        RuntimeSaveProfileRoute::CreateDisposable => create_operation(request, operation_id),
        RuntimeSaveProfileRoute::Lookup => Err("save_profile_route_invalid"),
    }
}

fn select_operation(
    request: &HttpRequest,
) -> Result<(SaveProfileOperation, Vec<u8>), &'static str> {
    let value = parse_body(request)?;
    let object = value.as_object().ok_or("save_profile_body_invalid")?;
    let has_primary = object.contains_key("profile_id");
    let has_alias = object.contains_key("save_profile_id");
    if object.len() != 2 || has_primary == has_alias || !object.contains_key("baseline") {
        return Err("save_profile_body_fields_invalid");
    }
    let profile = object
        .get("profile_id")
        .or_else(|| object.get("save_profile_id"))
        .and_then(Value::as_str)
        .ok_or("save_profile_profile_required")?;
    let profile_id = sts2_gateway::SaveProfileId::try_new(profile.to_owned())
        .map_err(|_| "save_profile_profile_invalid")?;
    let baseline = object
        .get("baseline")
        .ok_or("save_profile_baseline_required")
        .and_then(|value| {
            serde_json::from_value(value.clone()).map_err(|_| "save_profile_baseline_invalid")
        })?;
    Ok((
        SaveProfileOperation::Select {
            profile_id,
            baseline,
        },
        request.body.clone(),
    ))
}

fn create_operation(
    request: &HttpRequest,
    operation_id: &str,
) -> Result<(SaveProfileOperation, Vec<u8>), &'static str> {
    let body = if request.body.is_empty() {
        b"{}".to_vec()
    } else {
        let value = parse_body(request)?;
        if !value.as_object().is_some_and(|object| object.is_empty()) {
            return Err("save_profile_body_fields_invalid");
        }
        request.body.clone()
    };
    let instance_id = request
        .headers
        .get("x-sts2-instance-id")
        .cloned()
        .ok_or("save_profile_identity_missing")?;
    let placeholder = UserDataDescriptor {
        identity: UserDataIdentity::new(1),
        provenance: UserDataProvenance {
            owner: String::from("gateway"),
            instance_id,
            operation_id: operation_id.to_owned(),
            contract: sts2_gateway::LAUNCH_PROFILE_CONTRACT.to_owned(),
        },
        baseline: None,
    };
    let binding = LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, placeholder.identity)
        .map_err(|_| "save_profile_launch_profile_invalid")?;
    Ok((
        SaveProfileOperation::CreateDisposable {
            launch_profile: binding,
            user_data: placeholder,
        },
        body,
    ))
}

fn parse_body(request: &HttpRequest) -> Result<Value, &'static str> {
    if request.body.is_empty() || request.body.len() > MAX_BODY_BYTES {
        return Err("save_profile_body_invalid");
    }
    super::super::strict_json::parse(&request.body).map_err(|_| "save_profile_body_invalid")
}

pub(super) fn add_creation_binding(
    body: Vec<u8>,
    descriptor: &UserDataDescriptor,
    binding: &LaunchProfileBinding,
) -> Result<Vec<u8>, &'static str> {
    let mut object = super::super::strict_json::parse(&body)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or("save_profile_body_invalid")?;
    let descriptor =
        serde_json::to_value(descriptor).map_err(|_| "save_profile_encoding_failed")?;
    let binding = serde_json::to_value(binding).map_err(|_| "save_profile_encoding_failed")?;
    object.insert(String::from("user_data"), descriptor);
    object.insert(String::from("launch_profile"), binding);
    serde_json::to_vec(&Value::Object(object)).map_err(|_| "save_profile_encoding_failed")
}
