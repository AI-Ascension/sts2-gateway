// SPDX-License-Identifier: MIT

use super::*;
use sts2_gateway::{
    SaveProfileAuthority, SaveProfileContext, SaveProfileForwardRequest, SaveProfileLedger,
    UserDataProvisioner,
};

use super::super::save_profile::RuntimeSaveProfileRoute;
use super::super::save_profile_forwarder::HttpSaveProfileForwarder;
use super::service_save_profile_composition::{
    SaveProfileActiveRun, SaveProfileAllocation, SaveProfileDependencies, SaveProfileIntentStore,
};
use super::service_save_profile_creation::prepare_creation;
use super::service_save_profile_request::{
    build_operation, context_from_request, create_body, operation_id,
};
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
    ledger: SaveProfileLedger<HttpSaveProfileForwarder, SaveProfileIntentStore>,
    /// True only when an injected durable store backs the ledger. A mutation is refused before
    /// dispatch when the ledger would otherwise retain its intent in volatile storage.
    durable_intents: bool,
    /// Injected isolated-allocation, launch-profile binding, and allocation-record adapters.
    allocation: Option<SaveProfileAllocation>,
}

impl SaveProfileRuntime {
    /// Attached-runtime composition.
    ///
    /// No accepted durable intent store, isolated-allocation port, or launch-profile binding
    /// port is configured, so every mutation fails closed rather than provisioning from
    /// volatile in-memory stand-ins.
    pub(super) fn new(
        enabled: bool,
        mod_address: &str,
        mod_token: &str,
        authority: SaveProfileAuthority,
        capacity: usize,
    ) -> Result<Self, String> {
        Self::with_dependencies(
            enabled,
            mod_address,
            mod_token,
            authority,
            capacity,
            SaveProfileDependencies::unavailable(),
        )
    }

    pub(super) fn with_dependencies(
        enabled: bool,
        mod_address: &str,
        mod_token: &str,
        authority: SaveProfileAuthority,
        capacity: usize,
        dependencies: SaveProfileDependencies,
    ) -> Result<Self, String> {
        let SaveProfileDependencies {
            intents,
            allocation_port,
            allocation_intents,
            launch_profile,
        } = dependencies;
        let store = SaveProfileIntentStore::from_injected(intents);
        let durable_intents = store.durable();
        let ledger = SaveProfileLedger::new(
            capacity,
            authority,
            HttpSaveProfileForwarder::new(mod_address, mod_token),
            store,
        )
        .map_err(|error| format!("save-profile ledger is invalid: {error:?}"))?;
        let allocation = match (allocation_port, allocation_intents, launch_profile) {
            (None, None, None) => None,
            (Some(port), Some(intents), Some(launch_profile)) => Some(
                UserDataProvisioner::new(capacity, port, intents, launch_profile)
                    .map_err(|error| format!("user-data provisioner is invalid: {error:?}"))?,
            ),
            _ => {
                return Err(String::from(
                    "save-profile allocation adapters must be injected together",
                ));
            }
        };
        Ok(Self {
            enabled,
            ledger,
            durable_intents,
            allocation,
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
        active_run: SaveProfileActiveRun,
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
        if route == RuntimeSaveProfileRoute::CreateDisposable {
            let body = match create_body(request) {
                Ok(body) => body,
                Err(code) => return (400, json_error(code)),
            };
            if let Err(response) = self.admit_mutation(active_run) {
                return response;
            }
            return self.create_and_forward(
                context,
                operation_id,
                body,
                authority,
                active_run,
                now_millis,
            );
        }
        let (operation, body) = match build_operation(route, request) {
            Ok(value) => value,
            Err(code) => return (400, json_error(code)),
        };
        if route.is_mutation()
            && let Err(response) = self.admit_mutation(active_run)
        {
            return response;
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

    /// Fails closed whenever the composition has no authoritative active-run source or no
    /// durable operation-intent store.
    fn admit_mutation(&self, active_run: SaveProfileActiveRun) -> Result<(), (u16, Vec<u8>)> {
        if active_run.active() {
            return Err((409, json_error("save_profile_active_run")));
        }
        if !active_run.configured() {
            return Err((503, json_error("save_profile_active_run_unavailable")));
        }
        if !self.durable_intents {
            return Err((503, json_error("save_profile_persistence_unavailable")));
        }
        Ok(())
    }

    fn create_and_forward(
        &mut self,
        context: SaveProfileContext,
        operation_id: String,
        body: Vec<u8>,
        authority: SaveProfileAuthority,
        active_run: SaveProfileActiveRun,
        now_millis: u64,
    ) -> (u16, Vec<u8>) {
        let Some(allocation) = self.allocation.as_mut() else {
            return (503, json_error("save_profile_provisioning_unavailable"));
        };
        let request = match prepare_creation(allocation, context, operation_id, body) {
            Ok(request) => request,
            Err(response) => return response,
        };
        self.submit(request, authority, active_run, now_millis)
    }

    fn submit(
        &mut self,
        request: SaveProfileForwardRequest,
        authority: SaveProfileAuthority,
        active_run: SaveProfileActiveRun,
        now_millis: u64,
    ) -> (u16, Vec<u8>) {
        self.ledger.set_authority(authority);
        let ledger_active_run = active_run.active();
        match self.ledger.submit(request, now_millis, ledger_active_run) {
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
        let provisioned = self
            .ledger
            .operation(&context.instance_id, &operation_id)
            .is_none()
            && self.allocation.as_ref().is_some_and(|allocation| {
                allocation
                    .operation(&context.instance_id, &operation_id)
                    .is_some()
            });
        if provisioned && let Some(allocation) = self.allocation.as_mut() {
            return match allocation.reconcile(&context, &operation_id) {
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

fn safe_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains('/')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
