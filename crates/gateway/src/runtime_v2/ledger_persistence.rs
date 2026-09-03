// SPDX-License-Identifier: MIT

impl<P> RuntimeV2Ledger<P>
where
    P: RuntimeV2ForwardingPort,
{
    /// Returns the gateway-owned state required to reconstruct this ledger after a restart.
    #[must_use]
    pub fn persisted_state(&self) -> RuntimeV2PersistedState {
        RuntimeV2PersistedState {
            instance_id: self.binding.instance_id.clone(),
            session_id: self.binding.session_id.clone(),
            lease_id: self.binding.lease_id.clone(),
            lease_epoch: self.binding.lease_epoch,
            observation: self.binding.observation,
            operations: self
                .operations
                .values()
                .map(|operation| RuntimeV2PersistedOperation {
                    request: operation.request.clone(),
                    result: operation.result.clone(),
                })
                .collect(),
        }
    }

    /// Restores state, converting in-flight or merely accepted work to explicit `unknown`.
    pub fn restore_state(
        &mut self,
        persisted: RuntimeV2PersistedState,
    ) -> Result<(), RuntimeV2LedgerError> {
        if persisted.instance_id != self.binding.instance_id
            || persisted.session_id != self.binding.session_id
            || persisted.lease_id != self.binding.lease_id
            || persisted.lease_epoch != self.binding.lease_epoch
        {
            return Err(RuntimeV2LedgerError::PersistedStateMismatch);
        }
        persisted
            .observation
            .validate()
            .map_err(|_| RuntimeV2LedgerError::PersistedStateInvalid)?;
        if persisted.operations.len() > self.config.operation_capacity {
            return Err(RuntimeV2LedgerError::PersistedStateInvalid);
        }

        let mut restored = BTreeMap::new();
        for persisted_operation in persisted.operations {
            let request = persisted_operation.request;
            self.validate_action_request(&request)
                .map_err(|_| RuntimeV2LedgerError::PersistedStateInvalid)?;
            self.validate_context(&request)
                .map_err(|_| RuntimeV2LedgerError::PersistedStateInvalid)?;
            let key = self
                .key_for(&request)
                .map_err(|_| RuntimeV2LedgerError::PersistedStateInvalid)?;
            if restored.contains_key(&key) {
                return Err(RuntimeV2LedgerError::PersistedStateInvalid);
            }
            let canonical_request = request
                .idempotency_canonical_json()
                .map_err(|_| RuntimeV2LedgerError::PersistedStateInvalid)?;
            let request_digest = request
                .request_digest()
                .map_err(|_| RuntimeV2LedgerError::PersistedStateInvalid)?;

            let result = match persisted_operation.result {
                None => self.restart_uncertain_response(&request)?,
                Some(result) => {
                    if !self.response_matches_request(&request, &result)
                        || result.validate().is_err()
                        || result.kind != RuntimeV2MessageKind::ActionResponse
                    {
                        return Err(RuntimeV2LedgerError::PersistedStateInvalid);
                    }
                    if result.status == Some(RuntimeV2Status::Accepted) {
                        self.restart_uncertain_response(&request)?
                    } else {
                        result
                    }
                }
            };
            restored.insert(
                key,
                RuntimeV2Operation {
                    request_digest,
                    canonical_request,
                    request,
                    result: Some(result),
                },
            );
        }

        self.binding.observation = persisted.observation;
        self.operations = restored;
        Ok(())
    }

    fn restart_uncertain_response(
        &self,
        request: &RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        self.result_response(
            request,
            RuntimeV2Status::Unknown,
            None,
            Some(String::from("sts2.runtime/restart_uncertain")),
            None,
            RuntimeV2MessageKind::ActionResponse,
        )
    }

    /// Submits one fixed action, retaining its result before returning it.
    pub fn submit_action(
        &mut self,
        request: RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        self.submit_action_with_checkpoint(request, |_| Ok(()))
    }

    /// Submits one action with owner-managed durable checkpoints around dispatch.
    pub fn submit_action_with_checkpoint<F>(
        &mut self,
        request: RuntimeV2Message,
        mut checkpoint: F,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError>
    where
        F: FnMut(&RuntimeV2PersistedState) -> Result<(), ()>,
    {
        self.validate_action_request(&request)?;
        let key = self.key_for(&request)?;
        let canonical_request = request
            .idempotency_canonical_json()
            .map_err(RuntimeV2LedgerError::RequestDigest)?;
        let digest = request
            .request_digest()
            .map_err(RuntimeV2LedgerError::RequestDigest)?;
        self.validate_context(&request)?;
        if let Some(existing) = self.operations.get(&key) {
            return self.replay_or_conflict(existing, &request, digest, &canonical_request);
        }
        if self.operations.len() >= self.config.operation_capacity {
            return Err(RuntimeV2LedgerError::CapacityExceeded);
        }

        if request.generation != self.binding.observation.generation {
            let response = self.rejected_response(
                &request,
                "sts2.game-core/stale_generation",
                self.binding.observation,
            )?;
            self.retain(key.clone(), digest, canonical_request, request, response.clone());
            self.checkpoint_or_remove(&key, &mut checkpoint)?;
            return Ok(response);
        }

        let observation = self.binding.observation;
        if !observation.host_ready {
            let response =
                self.rejected_response(&request, "sts2.runtime/host_not_ready", observation)?;
            self.retain(key.clone(), digest, canonical_request, request, response.clone());
            self.checkpoint_or_remove(&key, &mut checkpoint)?;
            return Ok(response);
        }
        let phase_error = match observation.combat_phase {
            RuntimeV2CombatPhase::OutsideCombat => Some("sts2.game-core/outside_combat"),
            RuntimeV2CombatPhase::EnemyTurn => Some("sts2.game-core/not_player_turn"),
            RuntimeV2CombatPhase::PlayerTurn => None,
        };
        if let Some(error_code) = phase_error {
            let response = self.rejected_response(&request, error_code, observation)?;
            self.retain(key.clone(), digest, canonical_request, request, response.clone());
            self.checkpoint_or_remove(&key, &mut checkpoint)?;
            return Ok(response);
        }

        self.operations.insert(
            key.clone(),
            RuntimeV2Operation {
                request_digest: digest,
                canonical_request,
                request: request.clone(),
                result: None,
            },
        );
        if checkpoint(&self.persisted_state()).is_err() {
            self.operations.remove(&key);
            return Err(RuntimeV2LedgerError::PersistenceFailed);
        }
        let response = match self
            .forwarding
            .forward_runtime_v2(RuntimeV2ForwardRequest::new(request.clone()))
        {
            Ok(response) => self.accept_forwarded_response(&request, response),
            Err(fault) => self.response_for_transport_fault(&request, fault),
        }?;
        if let Some(operation) = self.operations.get_mut(&key) {
            operation.result = Some(response.clone());
        }
        if checkpoint(&self.persisted_state()).is_err() {
            let uncertain = self.persistence_uncertain_response(&request)?;
            if let Some(operation) = self.operations.get_mut(&key) {
                operation.result = Some(uncertain.clone());
            }
            return Ok(uncertain);
        }
        Ok(response)
    }

    fn checkpoint_or_remove<F>(
        &mut self,
        key: &RuntimeV2OperationKey,
        checkpoint: &mut F,
    ) -> Result<(), RuntimeV2LedgerError>
    where
        F: FnMut(&RuntimeV2PersistedState) -> Result<(), ()>,
    {
        if checkpoint(&self.persisted_state()).is_err() {
            self.operations.remove(key);
            return Err(RuntimeV2LedgerError::PersistenceFailed);
        }
        Ok(())
    }

    fn persistence_uncertain_response(
        &self,
        request: &RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        self.result_response(
            request,
            RuntimeV2Status::Unknown,
            None,
            Some(String::from("sts2.runtime/persistence_uncertain")),
            None,
            RuntimeV2MessageKind::ActionResponse,
        )
    }
}
