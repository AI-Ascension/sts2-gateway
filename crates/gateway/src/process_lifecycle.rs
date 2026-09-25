// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::identity::{AuthorityEpoch, InstanceId, Lease, LeaseProof, OperationId, Tick};
use crate::process_profile::ApprovedLaunchProfiles;
use crate::process_store::{LifecycleOperation, LifecycleRecordKey, LifecycleRecordStore};
use crate::{
    Clock, LeaseDecisionPort, LifecycleError, LifecycleRequest, LifecycleResponse,
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
    pub(crate) ownership: BTreeMap<InstanceId, crate::LifecycleOwnership>,
    /// Compatibility mirror for callers that inspect the currently known
    /// identity. The durable `ownership` map is authoritative.
    pub(crate) owned: BTreeMap<InstanceId, crate::ProcessIdentity>,
    pub(crate) next_sequence: u64,
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
        if config.max_processes() == 0 || config.max_records() == 0 {
            return Err(LifecycleError::CapacityExceeded);
        }
        if profiles.capacity() == 0 {
            return Err(LifecycleError::Profile(
                crate::LaunchProfileError::CapacityExceeded,
            ));
        }
        if store.count()? > config.max_records() {
            return Err(LifecycleError::CapacityExceeded);
        }
        let persisted = store.list()?;
        let persisted_ownership = store.list_ownership()?;
        let mut records = BTreeMap::new();
        let mut authority = BTreeMap::new();
        let mut next_sequence = 0;
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
            next_sequence = next_sequence.max(operation.sequence());
            records.insert(key, operation);
        }
        let mut lifecycle = Self {
            config,
            profiles,
            clock,
            process,
            store,
            fence,
            leases: BTreeMap::new(),
            authority,
            records,
            ownership: persisted_ownership
                .into_iter()
                .map(|owner| (owner.instance_id(), owner))
                .collect(),
            owned: BTreeMap::new(),
            next_sequence,
        };
        lifecycle.bootstrap_ownership()?;
        Ok(lifecycle)
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

    /// Binds the already-authenticated lease context of an attached adapter.
    ///
    /// The attached runtime authenticates every lifecycle request with the same
    /// instance/caller/session/lease/epoch fence it passes here, so this method grants no new
    /// authority: it converts a proof the caller already holds into the lease value
    /// `Self::authenticate` compares against. A caller cannot obtain a [`LeaseProof`] without
    /// first presenting that identity fence, and the proof's own fence is re-checked on every
    /// operation.
    ///
    /// `expires_at` is explicit because the attached adapter's lease liveness is its own
    /// `lease_active` gate rather than a wall-clock deadline; an adapter without an independent
    /// expiry states that here instead of having one invented for it.
    pub fn bind_attached_lease(
        &mut self,
        proof: LeaseProof,
        expires_at: Tick,
    ) -> Result<(), LifecycleError> {
        self.bind_lease(Lease::new(
            proof.instance_id(),
            proof.caller_id(),
            proof.session_id(),
            proof.lease_id(),
            proof.epoch(),
            expires_at,
        ))
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
        if !self.records.contains_key(&key) && self.records.len() >= self.config.max_records() {
            return Err(LifecycleError::CapacityExceeded);
        }
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

    pub(crate) fn set_authority_epoch(&mut self, instance_id: InstanceId, epoch: AuthorityEpoch) {
        self.authority.insert(instance_id, epoch);
    }

    pub(crate) fn config(&self) -> ProcessLifecycleConfig {
        self.config
    }
}
