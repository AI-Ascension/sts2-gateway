// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::super::runtime_v4_expert_rest_action_semantics::SelectorAdmission;
use super::RuntimeV4ExpertRestActionForwarder;

// Keep enough active bindings for a complete native campaign burst while still
// bounding retained request state. Terminal receipts are reclaimed first.
pub(crate) const MAX_OPERATION_BINDINGS: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OperationBinding {
    pub(super) action: Value,
    pub(super) generation: u64,
    pub(super) state_id: String,
    pub(super) instance_id: String,
    pub(super) session_id: String,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) status: OperationStatus,
    pub(super) completed_selector: Option<(String, SelectorAdmission)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OperationStatus {
    Pending,
    Accepted,
    Unknown,
    Settled,
    Rejected,
    Cancelled,
}

impl OperationStatus {
    pub(super) fn from_response(value: &Value) -> Option<Self> {
        match value["status"].as_str() {
            Some("accepted") => Some(Self::Accepted),
            Some("unknown") => Some(Self::Unknown),
            Some("settled") => Some(Self::Settled),
            Some("rejected") => Some(Self::Rejected),
            Some("cancelled") => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub(super) const fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Rejected | Self::Cancelled)
    }
}

impl RuntimeV4ExpertRestActionForwarder {
    pub(super) fn retain_operation_binding(&mut self, operation_id: &str, value: &Value) -> bool {
        if self.operation_bindings.len() >= MAX_OPERATION_BINDINGS {
            let terminal_id = self
                .operation_bindings
                .iter()
                .find_map(|(id, binding)| binding.status.is_terminal().then_some(id.clone()));
            let Some(terminal_id) = terminal_id else {
                return false;
            };
            self.operation_bindings.remove(&terminal_id);
        }
        let Some(generation) = value["generation"].as_u64() else {
            return false;
        };
        let Some(state_id) = value["state_id"].as_str() else {
            return false;
        };
        let Some(instance_id) = value["instance_id"].as_str() else {
            return false;
        };
        let Some(session_id) = value["session_id"].as_str() else {
            return false;
        };
        let Some(lease_id) = value["lease_id"].as_str() else {
            return false;
        };
        let Some(lease_epoch) = value["lease_epoch"].as_u64() else {
            return false;
        };
        self.operation_bindings.insert(
            operation_id.to_owned(),
            OperationBinding {
                action: value["action"].clone(),
                generation,
                state_id: state_id.to_owned(),
                instance_id: instance_id.to_owned(),
                session_id: session_id.to_owned(),
                lease_id: lease_id.to_owned(),
                lease_epoch,
                status: OperationStatus::Pending,
                completed_selector: None,
            },
        );
        true
    }

    pub(super) fn reconciliation_binding_valid(
        &self,
        route: &RuntimeV4ExpertRestActionRoute,
        value: &Value,
    ) -> bool {
        let Some(operation_id) = route.operation_id() else {
            return true;
        };
        let Some(binding) = self.operation_bindings.get(operation_id) else {
            return false;
        };
        if value["instance_id"] != binding.instance_id
            || value["session_id"] != binding.session_id
            || value["lease_id"] != binding.lease_id
            || value["lease_epoch"].as_u64() != Some(binding.lease_epoch)
        {
            return false;
        }
        match value["status"].as_str() {
            Some("settled") => {
                value["transition"]["before_generation"].as_u64() == Some(binding.generation)
            }
            Some("accepted" | "rejected" | "unknown" | "cancelled") => {
                value["generation"].as_u64() == Some(binding.generation)
                    && value["state_id"] == binding.state_id
            }
            _ => false,
        }
    }

    pub(super) fn record_operation_status(
        &mut self,
        route: &RuntimeV4ExpertRestActionRoute,
        request: &Value,
        value: &Value,
    ) {
        let Some(operation_id) = route
            .operation_id()
            .or_else(|| request["operation_id"].as_str())
        else {
            return;
        };
        let Some(status) = OperationStatus::from_response(value) else {
            return;
        };
        if let Some(binding) = self.operation_bindings.get_mut(operation_id) {
            binding.status = status;
        }
    }
}
