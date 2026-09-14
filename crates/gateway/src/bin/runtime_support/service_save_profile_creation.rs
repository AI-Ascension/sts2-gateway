// SPDX-License-Identifier: MIT

use super::*;
use sts2_gateway::{
    LAUNCH_PROFILE_ID, SaveProfileContext, SaveProfileForwardRequest, SaveProfileOperation,
    SaveProfileRoute,
};

use super::service_save_profile_composition::SaveProfileAllocation;
use super::service_save_profile_request::add_creation_binding;
use super::service_save_profile_wire::{provisioning_error, provisioning_outcome};

/// Provisions the reserved isolated allocation and binds it to the approved launch profile.
///
/// The launch binding always comes from the injected `LaunchProfileBindingPort` retained with
/// the allocation; this seam never builds a binding locally and never forwards a caller-supplied
/// path, command, or profile root.
pub(super) fn prepare_creation(
    allocation: &mut SaveProfileAllocation,
    context: SaveProfileContext,
    operation_id: String,
    body: Vec<u8>,
) -> Result<SaveProfileForwardRequest, (u16, Vec<u8>)> {
    let known = allocation
        .operation(&context.instance_id, &operation_id)
        .is_some();
    let provisioned = match if known {
        allocation.reconcile(&context, &operation_id)
    } else {
        allocation.create_disposable(context.clone(), operation_id.clone(), LAUNCH_PROFILE_ID)
    } {
        Ok(value) => value,
        Err(error) => return Err(provisioning_error(error)),
    };
    let Some(descriptor) = provisioned.descriptor.clone() else {
        return Err(provisioning_outcome(provisioned));
    };
    let Some(record) = allocation.operation(&context.instance_id, &operation_id) else {
        return Err((503, json_error("save_profile_launch_profile_unavailable")));
    };
    let binding = record.launch_profile.clone();
    if binding.validate().is_err() || binding.user_data != descriptor.identity {
        return Err((500, json_error("save_profile_launch_profile_invalid")));
    }
    let body = match add_creation_binding(body, &descriptor, &binding) {
        Ok(body) => body,
        Err(code) => return Err((500, json_error(code))),
    };
    Ok(SaveProfileForwardRequest {
        operation_id,
        context,
        route: SaveProfileRoute::CreateDisposable,
        operation: SaveProfileOperation::CreateDisposable {
            launch_profile: binding,
            user_data: descriptor,
        },
        body,
    })
}
