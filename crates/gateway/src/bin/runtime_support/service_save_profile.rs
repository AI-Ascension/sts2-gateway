// SPDX-License-Identifier: MIT

use super::*;
use serde_json::Value;
use sts2_gateway::{
    InMemorySaveProfileRecordStore, InMemoryUserDataPort, InMemoryUserDataRecordStore,
    LAUNCH_PROFILE_ID, LaunchProfileBinding, SaveProfileAuthority, SaveProfileContext,
    SaveProfileForwardRequest, SaveProfileLedger, SaveProfileOperation, SaveProfileRoute,
    UserDataProvisioner,
};

use super::super::save_profile::RuntimeSaveProfileRoute;
use super::super::save_profile_forwarder::HttpSaveProfileForwarder;
use super::service_save_profile_request::{add_creation_binding, build_operation};
use super::service_save_profile_wire::{
    ledger_error, provisioning_error, provisioning_outcome, result_response,
};

impl RuntimeService {
    pub(super) fn save_profile_request(
        &mut self,
        request: &HttpRequest,
        route: RuntimeSaveProfileRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        let authority = SaveProfileAuthority {
            instance_id: self.config.instance_id.clone(),
            caller_id: self.config.caller_id.clone(),
            session_id: self.config.session_id.clone(),
            lease_id: self.config.lease_id.clone(),
            lease_epoch: self.config.lease_epoch,
            expires_at_millis: None,
        };
        self.save_profile.request(
            route,
            request,
            authority,
            self.save_profile_active_run,
            unix_millis(),
        )
    }
}

pub(super) struct SaveProfileRuntime {
    enabled: bool,
    ledger: SaveProfileLedger<HttpSaveProfileForwarder, InMemorySaveProfileRecordStore>,
    provisioner: UserDataProvisioner<InMemoryUserDataPort, InMemoryUserDataRecordStore>,
}

struct CreateDisposableInput {
    context: SaveProfileContext,
    operation_id: String,
    body: Vec<u8>,
    authority: SaveProfileAuthority,
    active_run: bool,
    now_millis: u64,
}

impl SaveProfileRuntime {
    pub(super) fn new(
        enabled: bool,
        mod_address: &str,
        mod_token: &str,
        authority: SaveProfileAuthority,
        capacity: usize,
    ) -> Result<Self, String> {
        let ledger = SaveProfileLedger::new(
            capacity,
            authority,
            HttpSaveProfileForwarder::new(mod_address, mod_token),
            InMemorySaveProfileRecordStore::default(),
        )
        .map_err(|error| format!("save-profile ledger is invalid: {error:?}"))?;
        let provisioner = UserDataProvisioner::new(
            capacity,
            InMemoryUserDataPort::default(),
            Default::default(),
        )
        .map_err(|error| format!("user-data provisioner is invalid: {error:?}"))?;
        Ok(Self {
            enabled,
            ledger,
            provisioner,
        })
    }

    #[cfg(test)]
    pub(super) fn forwarding_mut(&mut self) -> &mut HttpSaveProfileForwarder {
        self.ledger.forwarding_mut()
    }

    pub(super) fn request(
        &mut self,
        route: RuntimeSaveProfileRoute,
        request: &HttpRequest,
        authority: SaveProfileAuthority,
        active_run: bool,
        now_millis: u64,
    ) -> (u16, Vec<u8>) {
        if !self.enabled {
            return (503, json_error("save_profile_unavailable"));
        }
        if route == RuntimeSaveProfileRoute::Lookup && !request.body.is_empty() {
            return (400, json_error("save_profile_body_not_allowed"));
        }
        if route.is_mutation() && !request.body.is_empty() && !request.content_type_is_json() {
            return (400, json_error("save_profile_content_type_invalid"));
        }
        let context = match context_from_request(request) {
            Ok(context) => context,
            Err(code) => return (400, json_error(code)),
        };
        let operation_id = operation_id(request, route);
        if operation_id.is_empty() || !safe_operation_id(&operation_id) {
            return (400, json_error("save_profile_operation_invalid"));
        }
        if route == RuntimeSaveProfileRoute::Lookup {
            return self.lookup(context, operation_id, authority, now_millis);
        }
        let (operation, body) = match build_operation(route, request, &operation_id) {
            Ok(value) => value,
            Err(code) => return (400, json_error(code)),
        };
        if route == RuntimeSaveProfileRoute::CreateDisposable {
            return self.create_and_forward(CreateDisposableInput {
                context,
                operation_id,
                body,
                authority,
                active_run,
                now_millis,
            });
        }
        let request = SaveProfileForwardRequest {
            operation_id,
            context,
            route: route.contract_route(),
            operation,
            body,
        };
        self.submit(request, authority, active_run, now_millis)
    }

    fn create_and_forward(&mut self, input: CreateDisposableInput) -> (u16, Vec<u8>) {
        if input.active_run {
            return (409, json_error("save_profile_active_run"));
        }
        let known = self
            .provisioner
            .operation(&input.context.instance_id, &input.operation_id)
            .is_some();
        let provisioned = match if known {
            self.provisioner
                .reconcile(&input.context, &input.operation_id)
        } else {
            self.provisioner.create_disposable(
                input.context.clone(),
                input.operation_id.clone(),
                LAUNCH_PROFILE_ID,
            )
        } {
            Ok(value) => value,
            Err(error) => return provisioning_error(error),
        };
        let Some(descriptor) = provisioned.descriptor.clone() else {
            return provisioning_outcome(provisioned);
        };
        let binding = match LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, descriptor.identity) {
            Ok(binding) => binding,
            Err(_) => return (500, json_error("save_profile_launch_profile_invalid")),
        };
        let body = match add_creation_binding(input.body, &descriptor, &binding) {
            Ok(body) => body,
            Err(code) => return (500, json_error(code)),
        };
        let operation = SaveProfileOperation::CreateDisposable {
            launch_profile: binding,
            user_data: descriptor,
        };
        let request = SaveProfileForwardRequest {
            operation_id: input.operation_id,
            context: input.context,
            route: SaveProfileRoute::CreateDisposable,
            operation,
            body,
        };
        self.submit(request, input.authority, input.active_run, input.now_millis)
    }

    fn submit(
        &mut self,
        request: SaveProfileForwardRequest,
        authority: SaveProfileAuthority,
        active_run: bool,
        now_millis: u64,
    ) -> (u16, Vec<u8>) {
        self.ledger.set_authority(authority);
        match self.ledger.submit(request, now_millis, active_run) {
            Ok(result) => result_response(result),
            Err(error) => ledger_error(error),
        }
    }

    fn lookup(
        &mut self,
        context: SaveProfileContext,
        operation_id: String,
        authority: SaveProfileAuthority,
        now_millis: u64,
    ) -> (u16, Vec<u8>) {
        self.ledger.set_authority(authority);
        if self
            .ledger
            .operation(&context.instance_id, &operation_id)
            .is_none()
            && self
                .provisioner
                .operation(&context.instance_id, &operation_id)
                .is_some()
        {
            return match self.provisioner.reconcile(&context, &operation_id) {
                Ok(outcome) => provisioning_outcome(outcome),
                Err(error) => provisioning_error(error),
            };
        }
        match self.ledger.reconcile(context, &operation_id, now_millis) {
            Ok(result) => result_response(result),
            Err(error) => ledger_error(error),
        }
    }
}

fn context_from_request(request: &HttpRequest) -> Result<SaveProfileContext, &'static str> {
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

fn operation_id(request: &HttpRequest, route: RuntimeSaveProfileRoute) -> String {
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
