// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::super::runtime_v4_expert_rest_action_semantics::{
    SelectorAdmission, SelectorLifecycle, admission_from_transition,
};
use super::RuntimeV4ExpertRestActionForwarder;

// A selector admission is retained while its selection can still authorize a
// later action. Completed selectors reclaim their slot for long campaigns.
pub(crate) const MAX_SELECTOR_ADMISSIONS: usize = 128;

pub(super) fn request_needs_selector_reservation(value: &Value) -> bool {
    let action = &value["action"]["action"];
    action["kind"] == "rest_option"
        && matches!(action["rest_option_id"].as_str(), Some("smith" | "mend"))
}

impl RuntimeV4ExpertRestActionForwarder {
    pub(super) fn reserve_selector_admission(&mut self, operation_id: &str) -> bool {
        if self.selector_reservations.contains(operation_id) {
            return true;
        }
        if self.selector_admissions.len() + self.selector_reservations.len()
            >= MAX_SELECTOR_ADMISSIONS
        {
            return false;
        }
        self.selector_reservations.insert(operation_id.to_owned());
        true
    }

    pub(super) fn release_selector_reservation(&mut self, operation_id: &str) {
        self.selector_reservations.remove(operation_id);
    }

    pub(super) fn completed_selector_for<'a>(
        &'a self,
        route: &RuntimeV4ExpertRestActionRoute,
        request: &'a Value,
    ) -> Option<(&'a str, &'a SelectorAdmission)> {
        let operation_id = route
            .operation_id()
            .or_else(|| request["operation_id"].as_str())?;
        self.operation_bindings
            .get(operation_id)?
            .completed_selector
            .as_ref()
            .map(|(selection_id, admission)| (selection_id.as_str(), admission))
    }

    pub(super) fn record_selector_admission(
        &mut self,
        route: &RuntimeV4ExpertRestActionRoute,
        request: &Value,
        value: &Value,
    ) {
        let operation_id = route
            .operation_id()
            .or_else(|| request["operation_id"].as_str());
        let transition = value["transition"].as_object();
        let kind = transition
            .and_then(|transition| transition.get("kind"))
            .and_then(Value::as_str);
        match kind {
            Some("rest_option_selection_requested" | "rest_option_selection_progressed") => {
                let Some(lifecycle) = SelectorLifecycle::from_value(value) else {
                    return;
                };
                let Some((selection_id, admission)) = admission_from_transition(
                    &Value::Object(transition.cloned().unwrap_or_default()),
                    lifecycle,
                ) else {
                    return;
                };
                let operation_context = operation_id.and_then(|operation_id| {
                    self.operation_bindings
                        .get(operation_id)
                        .and_then(|binding| binding.selector_context.as_ref())
                        .cloned()
                });
                if self.completed_selector_blocks(&selection_id, &admission) {
                    if let Some(operation_id) = operation_id {
                        self.release_selector_reservation(operation_id);
                    }
                    return;
                }
                if self.selector_admissions.len() < MAX_SELECTOR_ADMISSIONS
                    || self.selector_admissions.contains_key(&selection_id)
                {
                    if let Some(operation_id) = operation_id {
                        self.release_selector_reservation(operation_id);
                    }
                    if let Some(operation_id) = operation_id
                        && operation_context.is_none()
                        && let Some(binding) = self.operation_bindings.get_mut(operation_id)
                    {
                        let context = self
                            .selector_admissions
                            .get(&selection_id)
                            .cloned()
                            .unwrap_or_else(|| admission.clone());
                        binding.selector_context = Some((selection_id.clone(), context));
                    }
                    let should_advance =
                        self.selector_admissions
                            .get(&selection_id)
                            .is_none_or(|current| {
                                (current.lifecycle == admission.lifecycle
                                    && admission.generation > current.generation)
                                    || (current.lifecycle != admission.lifecycle
                                        && !request.is_null())
                            });
                    if should_advance {
                        self.selector_admissions.insert(selection_id, admission);
                    }
                }
            }
            Some("rest_option_selection_completed") => {
                let Some(selection_id) = value["transition"]["selection_id"].as_str() else {
                    return;
                };
                let Some(lifecycle) = SelectorLifecycle::from_value(value) else {
                    return;
                };
                let Some(before_generation) = value["transition"]["before_generation"].as_u64()
                else {
                    return;
                };
                let operation_context = operation_id.and_then(|operation_id| {
                    self.operation_bindings
                        .get(operation_id)
                        .and_then(|binding| binding.selector_context.as_ref())
                        .cloned()
                });
                let current_matches_completion = self
                    .selector_admissions
                    .get(selection_id)
                    .is_some_and(|current| {
                        current.lifecycle == lifecycle
                            && current.generation == before_generation
                            && operation_context.as_ref().is_some_and(|(id, context)| {
                                id == selection_id && context == current
                            })
                    });
                if current_matches_completion
                    && let Some(admission) = self.selector_admissions.remove(selection_id)
                {
                    let completed = (selection_id.to_owned(), admission);
                    for binding in self.operation_bindings.values_mut() {
                        let same_lifecycle =
                            binding_lifecycle_matches(binding, &completed.1.lifecycle);
                        let related =
                            binding
                                .selector_context
                                .as_ref()
                                .is_some_and(|(id, context)| {
                                    id == selection_id && context.lifecycle == completed.1.lifecycle
                                })
                                || binding.completed_selector.as_ref().is_some_and(
                                    |(id, context)| {
                                        id == selection_id
                                            && context.lifecycle == completed.1.lifecycle
                                    },
                                )
                                || binding.action["action"]["selection_id"].as_str()
                                    == Some(selection_id)
                                    && same_lifecycle;
                        let compatible_tombstone = binding
                            .completed_selector
                            .as_ref()
                            .is_none_or(|(_, context)| context.lifecycle == completed.1.lifecycle);
                        if related && compatible_tombstone {
                            binding.completed_selector = Some(completed.clone());
                        }
                    }
                    if let Some(operation_id) = operation_id
                        && let Some(binding) = self.operation_bindings.get_mut(operation_id)
                    {
                        binding.completed_selector = Some(completed);
                    }
                }
            }
            _ => {}
        }
        if (matches!(value["status"].as_str(), Some("rejected" | "cancelled"))
            || (value["status"] == "settled"
                && !matches!(
                    kind,
                    Some("rest_option_selection_requested" | "rest_option_selection_progressed")
                )))
            && let Some(operation_id) = operation_id
        {
            self.release_selector_reservation(operation_id);
        }
    }

    fn completed_selector_blocks(&self, selection_id: &str, candidate: &SelectorAdmission) -> bool {
        self.operation_bindings.values().any(|binding| {
            binding
                .completed_selector
                .as_ref()
                .is_some_and(|(completed_id, tombstone)| {
                    completed_id == selection_id
                        && tombstone.lifecycle == candidate.lifecycle
                        && candidate.generation <= tombstone.generation
                })
        })
    }
}

fn binding_lifecycle_matches(
    binding: &super::operations::OperationBinding,
    lifecycle: &SelectorLifecycle,
) -> bool {
    binding.instance_id == lifecycle.instance_id
        && binding.session_id == lifecycle.session_id
        && binding.lease_id == lifecycle.lease_id
        && binding.lease_epoch == lifecycle.lease_epoch
}
