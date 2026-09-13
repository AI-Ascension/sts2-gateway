// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};

use crate::identity::{AuthorityEpoch, InstanceId, Lease, LeaseProof, OperationId};
use crate::process_profile::ApprovedLaunchProfiles;
use crate::process_store::{
    LifecycleOperation, LifecycleOperationState, LifecycleRecordKey, LifecycleRecordStore,
};
use crate::{
    Clock, LeaseDecisionPort, LifecycleError, LifecycleRequest, LifecycleResponse, ProcessIdentity,
    ProcessLifecycleConfig, ProcessPort,
};

/// Gateway-owned lifecycle state for profile-approved processes and their durable operations.
pub struct ProcessLifecycle<C, P, S, F> {
    pub(crate) config: ProcessLifecycleConfig,
    pub(crate) profiles: ApprovedLaunchProfiles,
    pub(crate) clock: C,
    pub(crate) process: P,
    pub(crate) store: S,
    pub(crate) fence: F,
    pub(crate) leases: BTreeMap<InstanceId, Lease>,
    pub(crate) authority: BTreeMap<InstanceId, AuthorityEpoch>,
    pub(crate) records: BTreeMap<LifecycleRecordKey, LifecycleOperation>,
    pub(crate) owned: BTreeMap<InstanceId, ProcessIdentity>,
}

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: Clock,
    P: ProcessPort,
    S: LifecycleRecordStore,
    F: LeaseDecisionPort,
{
    pub fn new(
        config: ProcessLifecycleConfig,
        profiles: ApprovedLaunchProfiles,
        clock: C,
        process: P,
        store: S,
        fence: F,
    ) -> Result<Self, LifecycleError> {
        if config.max_processes() == 0 {
            return Err(LifecycleError::CapacityExceeded);
        }
        if profiles.capacity() == 0 {
            return Err(LifecycleError::Profile(
                crate::LaunchProfileError::CapacityExceeded,
            ));
        }
        let persisted = store.list()?;
        let mut records = BTreeMap::new();
        let mut authority = BTreeMap::new();
        for operation in persisted {
            let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
            authority
                .entry(operation.instance_id())
                .and_modify(|current: &mut AuthorityEpoch| {
                    if operation.authority_epoch() > *current {
                        *current = operation.authority_epoch();
                    }
                })
                .or_insert(operation.authority_epoch());
            records.insert(key, operation);
        }
        let mut latest = BTreeMap::new();
        for operation in records.values() {
            latest
                .entry(operation.instance_id())
                .and_modify(|current: &mut LifecycleOperation| {
                    if operation.operation_id() > current.operation_id() {
                        *current = operation.clone();
                    }
                })
                .or_insert_with(|| operation.clone());
        }
        let mut owned = BTreeMap::new();
        for operation in latest.values() {
            if operation.state().is_active()
                && let Some(identity) = operation.process()
            {
                owned.insert(operation.instance_id(), identity.clone());
            }
        }
        Ok(Self {
            config,
            profiles,
            clock,
            process,
            store,
            fence,
            leases: BTreeMap::new(),
            authority,
            records,
            owned,
        })
    }

    pub fn bind_lease(&mut self, lease: Lease) -> Result<(), LifecycleError> {
        let instance_id = lease.instance_id();
        if let Some(current) = self.leases.get(&instance_id)
            && current.proof() != lease.proof()
        {
            return Err(LifecycleError::IdentityMismatch);
        }
        self.leases.insert(instance_id, lease);
        self.authority
            .entry(instance_id)
            .or_insert(AuthorityEpoch::new(1));
        Ok(())
    }

    pub fn authorize_lease(&mut self, lease: Lease) -> Result<(), LifecycleError> {
        self.bind_lease(lease)
    }

    pub fn apply(
        &mut self,
        request: LifecycleRequest,
    ) -> Result<LifecycleResponse, LifecycleError> {
        self.apply_request(request)
    }

    pub fn reconcile(
        &mut self,
        lease: LeaseProof,
        authority_epoch: AuthorityEpoch,
        operation_id: OperationId,
    ) -> Result<LifecycleResponse, LifecycleError> {
        self.reconcile_operation(lease, authority_epoch, operation_id)
    }

    pub fn operation(
        &self,
        instance_id: InstanceId,
        operation_id: OperationId,
    ) -> Option<&LifecycleOperation> {
        self.records
            .get(&LifecycleRecordKey::new(instance_id, operation_id))
    }

    pub fn operations(&self) -> impl Iterator<Item = &LifecycleOperation> {
        self.records.values()
    }

    pub fn current_authority_epoch(&self, instance_id: InstanceId) -> AuthorityEpoch {
        match self.authority.get(&instance_id) {
            Some(epoch) => *epoch,
            None => AuthorityEpoch::new(1),
        }
    }

    pub fn profiles(&self) -> &ApprovedLaunchProfiles {
        &self.profiles
    }

    pub fn process(&self) -> &P {
        &self.process
    }

    pub fn process_mut(&mut self) -> &mut P {
        &mut self.process
    }

    pub fn into_store(self) -> S {
        self.store
    }

    pub fn into_parts(self) -> (P, S) {
        (self.process, self.store)
    }

    pub(crate) fn authenticate(
        &mut self,
        instance_id: InstanceId,
        proof: LeaseProof,
        authority_epoch: AuthorityEpoch,
    ) -> Result<(), LifecycleError> {
        let Some(lease) = self.leases.get(&instance_id).copied() else {
            return Err(LifecycleError::LeaseNotBound);
        };
        self.fence
            .check_fence(Some(lease), instance_id, proof, self.clock.now())
            .map_err(LifecycleError::Fence)?;
        if self.current_authority_epoch(instance_id) != authority_epoch {
            return Err(LifecycleError::StaleAuthorityEpoch);
        }
        Ok(())
    }

    pub(crate) fn persist_insert(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<(), LifecycleError> {
        let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
        self.store.insert(operation.clone())?;
        self.records.insert(key, operation);
        Ok(())
    }

    pub(crate) fn persist_update(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<(), LifecycleError> {
        let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
        self.store.update(operation.clone())?;
        self.records.insert(key, operation);
        Ok(())
    }

    pub(crate) fn record_for(
        &self,
        instance_id: InstanceId,
        operation_id: OperationId,
    ) -> Option<LifecycleOperation> {
        self.operation(instance_id, operation_id).cloned()
    }

    pub(crate) fn latest_authorized(&self, instance_id: InstanceId) -> Option<LifecycleOperation> {
        self.latest_authorized_excluding(instance_id, None)
    }

    pub(crate) fn latest_authorized_excluding(
        &self,
        instance_id: InstanceId,
        excluded_operation: Option<OperationId>,
    ) -> Option<LifecycleOperation> {
        self.records
            .values()
            .filter(|operation| {
                operation.instance_id() == instance_id
                    && Some(operation.operation_id()) != excluded_operation
                    && self
                        .leases
                        .get(&instance_id)
                        .is_some_and(|lease| lease.proof() == operation.lease())
            })
            .max_by_key(|operation| operation.operation_id())
            .and_then(|operation| {
                (operation.process().is_some()
                    && matches!(
                        operation.state(),
                        LifecycleOperationState::Started
                            | LifecycleOperationState::Attached
                            | LifecycleOperationState::Stopping
                            | LifecycleOperationState::Restarting
                            | LifecycleOperationState::Unknown
                            | LifecycleOperationState::Blocked
                    ))
                .then(|| operation.clone())
            })
    }

    pub(crate) fn active_operation(&self, instance_id: InstanceId) -> Option<LifecycleOperation> {
        self.records
            .values()
            .filter(|operation| operation.instance_id() == instance_id)
            .max_by_key(|operation| operation.operation_id())
            .filter(|operation| operation.state().is_active())
            .cloned()
    }

    pub(crate) fn set_authority_epoch(&mut self, instance_id: InstanceId, epoch: AuthorityEpoch) {
        self.authority.insert(instance_id, epoch);
    }

    pub(crate) fn set_owned(&mut self, instance_id: InstanceId, identity: ProcessIdentity) {
        self.owned.insert(instance_id, identity);
    }

    pub(crate) fn clear_owned(&mut self, instance_id: InstanceId) {
        self.owned.remove(&instance_id);
    }

    pub(crate) fn occupied_count(&self) -> usize {
        let mut instances = BTreeSet::new();
        instances.extend(self.owned.keys().copied());
        for instance_id in self
            .records
            .values()
            .map(|operation| operation.instance_id())
        {
            if let Some(operation) = self
                .records
                .values()
                .filter(|operation| operation.instance_id() == instance_id)
                .max_by_key(|operation| operation.operation_id())
                && operation.state().is_active()
            {
                instances.insert(instance_id);
            }
        }
        instances.len()
    }

    pub(crate) fn config(&self) -> ProcessLifecycleConfig {
        self.config
    }
}
