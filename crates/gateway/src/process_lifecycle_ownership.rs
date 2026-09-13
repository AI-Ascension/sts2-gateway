// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::process_store::{LifecycleOperation, LifecycleOperationState, LifecycleOwnership};
use crate::{InstanceId, LaunchProfileId, LifecycleError, OperationId, ProcessIdentity};

use super::ProcessLifecycle;
use super::process_lifecycle_order::{operation_order, owner_order, ownership_sequence};

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn bootstrap_ownership(&mut self) -> Result<(), LifecycleError> {
        if self.ownership.len() > self.config.max_processes() {
            return Err(LifecycleError::CapacityExceeded);
        }
        let owner_sequence = self
            .ownership
            .values()
            .map(LifecycleOwnership::sequence)
            .max()
            .unwrap_or_default();
        self.next_sequence = self.next_sequence.max(owner_sequence);
        for owner in self.ownership.values() {
            if owner
                .process()
                .is_some_and(|process| process.instance_id() != owner.instance_id())
            {
                return Err(LifecycleError::Store(
                    crate::LifecycleStoreError::Serialization,
                ));
            }
        }
        let stale_owners = self
            .ownership
            .values()
            .filter_map(|owner| {
                let latest = self.latest_record(owner.instance_id())?;
                let terminal = matches!(
                    latest.state(),
                    LifecycleOperationState::Stopped | LifecycleOperationState::Failed
                );
                (terminal && operation_order(&latest) >= owner_order(owner))
                    .then_some((owner.instance_id(), owner.clone()))
            })
            .collect::<Vec<_>>();
        for (instance_id, owner) in stale_owners {
            self.store.clear_ownership_if(&owner)?;
            self.ownership.remove(&instance_id);
            self.owned.remove(&instance_id);
        }
        for owner in self.ownership.values() {
            if let Some(identity) = owner.process().cloned() {
                self.owned.insert(owner.instance_id(), identity);
            }
        }
        let mut candidates = BTreeMap::new();
        let instances = self
            .records
            .values()
            .map(|operation| operation.instance_id())
            .collect::<std::collections::BTreeSet<_>>();
        for instance_id in instances {
            if self.ownership.contains_key(&instance_id) {
                continue;
            }
            let Some(operation) = self.latest_authoritative_record(instance_id) else {
                continue;
            };
            if !operation.state().is_active() {
                continue;
            }
            let Some(profile_id) = self.profile_id_for_record(&operation) else {
                continue;
            };
            candidates.insert(
                instance_id,
                (
                    operation.operation_id(),
                    ownership_sequence(&operation),
                    profile_id,
                    operation.process().cloned(),
                ),
            );
        }
        for (instance_id, (operation_id, sequence, profile_id, process)) in candidates {
            self.persist_ownership(LifecycleOwnership::new(
                instance_id,
                operation_id,
                sequence,
                profile_id,
                process,
            ))?;
        }
        Ok(())
    }

    pub(crate) fn issue_sequence(&mut self) -> Result<u64, LifecycleError> {
        let Some(sequence) = self.next_sequence.checked_add(1) else {
            return Err(LifecycleError::AuthorityExhausted);
        };
        self.next_sequence = sequence;
        Ok(sequence)
    }

    pub(crate) fn reserve_ownership(
        &mut self,
        operation: &LifecycleOperation,
        profile_id: LaunchProfileId,
    ) -> Result<(), LifecycleError> {
        if !self.operation_can_update_owner(operation) {
            return Ok(());
        }
        self.persist_ownership(LifecycleOwnership::new(
            operation.instance_id(),
            operation.operation_id(),
            ownership_sequence(operation),
            profile_id,
            operation.process().cloned(),
        ))
    }

    pub(crate) fn set_owned(
        &mut self,
        operation: &LifecycleOperation,
        profile_id: LaunchProfileId,
        identity: ProcessIdentity,
    ) -> Result<(), LifecycleError> {
        if !self.operation_can_update_owner(operation) {
            return Ok(());
        }
        self.persist_ownership(LifecycleOwnership::new(
            operation.instance_id(),
            operation.operation_id(),
            ownership_sequence(operation),
            profile_id,
            Some(identity),
        ))
    }

    pub(crate) fn set_owner_process_for_operation(
        &mut self,
        operation: &LifecycleOperation,
        process: Option<ProcessIdentity>,
    ) -> Result<(), LifecycleError> {
        if !self.operation_can_update_owner(operation) {
            return Ok(());
        }
        let instance_id = operation.instance_id();
        let Some(current) = self.ownership.get(&instance_id).cloned() else {
            return Ok(());
        };
        self.persist_ownership(LifecycleOwnership::new(
            current.instance_id(),
            current.operation_id(),
            current.sequence(),
            current.profile_id(),
            process,
        ))
    }

    pub(crate) fn clear_owned_for(
        &mut self,
        operation: &LifecycleOperation,
    ) -> Result<(), LifecycleError> {
        if !self.operation_can_update_owner(operation) {
            return Ok(());
        }
        self.clear_owned(operation.instance_id())
    }

    pub(crate) fn clear_owned(&mut self, instance_id: InstanceId) -> Result<(), LifecycleError> {
        if !self.ownership.contains_key(&instance_id) {
            self.owned.remove(&instance_id);
            return Ok(());
        }
        let Some(current) = self.ownership.get(&instance_id).cloned() else {
            self.owned.remove(&instance_id);
            return Ok(());
        };
        self.store.clear_ownership_if(&current)?;
        self.ownership.remove(&instance_id);
        self.owned.remove(&instance_id);
        Ok(())
    }

    pub(crate) fn latest_authorized(&self, instance_id: InstanceId) -> Option<LifecycleOperation> {
        self.latest_authorized_excluding(instance_id, None)
    }

    pub(crate) fn latest_authorized_excluding(
        &self,
        instance_id: InstanceId,
        excluded_operation: Option<OperationId>,
    ) -> Option<LifecycleOperation> {
        if self.ownership.contains_key(&instance_id) {
            let operation = self.authoritative_operation(instance_id)?;
            return (Some(operation.operation_id()) != excluded_operation
                && operation.process().is_some()
                && operation.state().is_active()
                && self.lease_matches(&operation))
            .then_some(operation);
        }
        let latest = self.latest_authoritative_record(instance_id)?;
        if Some(latest.operation_id()) != excluded_operation {
            return latest
                .state()
                .is_active()
                .then_some(latest)
                .filter(|operation| {
                    operation.process().is_some() && self.lease_matches(operation)
                });
        }
        self.records
            .values()
            .filter(|operation| {
                operation.instance_id() == instance_id
                    && Some(operation.operation_id()) != excluded_operation
                    && self.lease_matches(operation)
                    && operation.process().is_some()
                    && operation.state().is_active()
            })
            .max_by_key(|operation| operation_order(operation))
            .cloned()
    }

    pub(crate) fn active_operation(&self, instance_id: InstanceId) -> Option<LifecycleOperation> {
        self.authoritative_operation(instance_id)
            .filter(|operation| operation.state().is_active())
            .or_else(|| {
                self.latest_authoritative_record(instance_id)
                    .filter(|operation| operation.state().is_active())
            })
    }

    pub(crate) fn occupied_count(&self) -> usize {
        let mut instances = self
            .ownership
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let record_instances = self
            .records
            .values()
            .map(|operation| operation.instance_id())
            .collect::<std::collections::BTreeSet<_>>();
        for instance_id in record_instances {
            if self
                .latest_authoritative_record(instance_id)
                .is_some_and(|operation| operation.state().is_active())
            {
                instances.insert(instance_id);
            }
        }
        instances.len()
    }

    fn authoritative_operation(&self, instance_id: InstanceId) -> Option<LifecycleOperation> {
        let owner = self.ownership.get(&instance_id)?;
        self.records
            .get(&crate::LifecycleRecordKey::new(
                instance_id,
                owner.operation_id(),
            ))
            .cloned()
    }

    fn latest_record(&self, instance_id: InstanceId) -> Option<LifecycleOperation> {
        self.records
            .values()
            .filter(|operation| operation.instance_id() == instance_id)
            .max_by_key(|operation| operation_order(operation))
            .cloned()
    }

    pub(crate) fn latest_authoritative_record(
        &self,
        instance_id: InstanceId,
    ) -> Option<LifecycleOperation> {
        self.records
            .values()
            .filter(|operation| {
                operation.instance_id() == instance_id
                    && operation.state() != LifecycleOperationState::Rejected
            })
            .max_by_key(|operation| operation_order(operation))
            .cloned()
    }

    fn persist_ownership(&mut self, ownership: LifecycleOwnership) -> Result<(), LifecycleError> {
        self.store.set_ownership(ownership.clone())?;
        self.next_sequence = self.next_sequence.max(ownership.sequence());
        if let Some(identity) = ownership.process().cloned() {
            self.owned.insert(ownership.instance_id(), identity);
        } else {
            self.owned.remove(&ownership.instance_id());
        }
        self.ownership.insert(ownership.instance_id(), ownership);
        Ok(())
    }
}
