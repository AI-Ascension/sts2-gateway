// SPDX-License-Identifier: MIT

use super::super::save_profile::RuntimeSaveProfileRoute;
use super::*;
use serde_json::Value;
use sts2_gateway::{
    LaunchProfileBinding, SaveProfileContext, SaveProfileOperation, UserDataDescriptor,
};

pub(super) fn build_operation(
    route: RuntimeSaveProfileRoute,
    request: &HttpRequest,
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
        RuntimeSaveProfileRoute::CreateDisposable | RuntimeSaveProfileRoute::Lookup => {
            Err("save_profile_route_invalid")
        }
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

/// Validates the disposable-creation body. The operation itself is built from the reserved
/// allocation in the creation seam, so no launch binding is constructed from caller input.
pub(super) fn create_body(request: &HttpRequest) -> Result<Vec<u8>, &'static str> {
    if request.body.is_empty() {
        return Ok(b"{}".to_vec());
    }
    let value = parse_body(request)?;
    if !value.as_object().is_some_and(|object| object.is_empty()) {
        return Err("save_profile_body_fields_invalid");
    }
    Ok(request.body.clone())
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

pub(super) fn context_from_request(
    request: &HttpRequest,
) -> Result<SaveProfileContext, &'static str> {
    let get = |name: &str| {
        request
            .headers
            .get(name)
            .cloned()
            .ok_or("save_profile_identity_missing")
    };
    Ok(SaveProfileContext {
        instance_id: get("x-sts2-instance-id")?,
        caller_id: get("x-sts2-caller-id")?,
        session_id: get("x-sts2-session-id")?,
        lease_id: get("x-sts2-lease-id")?,
        lease_epoch: get("x-sts2-lease-epoch")?
            .parse()
            .map_err(|_| "save_profile_epoch_invalid")?,
        correlation_id: get("x-sts2-correlation-id")?,
    })
}

pub(super) fn operation_id(request: &HttpRequest, route: RuntimeSaveProfileRoute) -> String {
    if route == RuntimeSaveProfileRoute::Lookup {
        let Some(instance_id) = request.headers.get("x-sts2-instance-id") else {
            return String::new();
        };
        return route
            .operation_id(&request.path, instance_id)
            .unwrap_or_default()
            .to_owned();
    }
    request
        .headers
        .get("x-mcp-request-id")
        .cloned()
        .or_else(|| {
            route
                .is_mutation()
                .then(|| {
                    super::super::strict_json::parse(&request.body)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("operation_id")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                })
                .flatten()
        })
        .or_else(|| request.headers.get("x-sts2-correlation-id").cloned())
        .unwrap_or_default()
}
