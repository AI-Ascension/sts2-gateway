// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    RecoveryAdmissionTicket, RecoveryHostFence, RecoveryLeaseProof, RecoveryOperationState,
    RecoveryStoreError, RecoveryTicketState, validate_digest, validate_uuid, validate_uuid_v4,
    validate_wire,
};
use super::super::{GatewayRecoveryStore, random_uuid};

impl GatewayRecoveryStore {
    /// Creates the durable host admission ticket after dispatch has been
    /// recorded. The caller must present the current fence so a queued ticket
    /// cannot outlive a host-fence replacement.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_admission_ticket(
        &mut self,
        proof: &RecoveryLeaseProof,
        fence: &RecoveryHostFence,
        instance_id: &str,
        operation_id: &str,
        payload_digest: &str,
        now_millis: u64,
        ttl_seconds: u64,
    ) -> Result<RecoveryAdmissionTicket, RecoveryStoreError> {
        validate_uuid("instance_id", instance_id)?;
        validate_uuid_v4("operation_id", operation_id)?;
        validate_digest("payload_digest", payload_digest)?;
        validate_wire(now_millis, "now_millis")?;
        validate_wire(ttl_seconds, "ticket_ttl_seconds")?;
        let lease = self.ensure_context(proof, now_millis)?;
        validate_fence(fence)?;
        let current = self.current_host_fence()?;
        if current != *fence
            || fence.instance_id != instance_id
            || fence.boot_id != proof.boot_id
            || fence.instance_incarnation != proof.instance_incarnation
            || fence.authority_generation != proof.authority_generation
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        if ttl_seconds == 0 || ttl_seconds > 300 {
            return Err(RecoveryStoreError::InvalidInput(
                "ticket TTL must be between 1 and 300 seconds".to_owned(),
            ));
        }
        let ttl_millis = ttl_seconds
            .checked_mul(1_000)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        let expires_at_millis = now_millis
            .checked_add(ttl_millis)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        validate_wire(expires_at_millis, "ticket_expires_at_millis")?;
        if expires_at_millis > lease.expires_at_millis {
            return Err(RecoveryStoreError::LeaseExpired);
        }

        let tx = self.transaction()?;
        let operation = super::operations::select_operation(&tx, instance_id, operation_id)?
            .ok_or(RecoveryStoreError::OperationNotFound)?;
        if operation.payload_digest != payload_digest
            || operation.boot_id != proof.boot_id
            || operation.instance_incarnation != proof.instance_incarnation
            || operation.lease_epoch != proof.lease_epoch
        {
            return Err(RecoveryStoreError::OperationConflict);
        }
        if operation.state != RecoveryOperationState::MayHaveBeenDispatched {
            return Err(RecoveryStoreError::InvalidTransition);
        }
        if let Some(ticket) = row_ticket_for_operation(&tx, instance_id, operation_id)? {
            if ticket.payload_digest == payload_digest {
                return Ok(ticket);
            }
            return Err(RecoveryStoreError::OperationConflict);
        }
        let ticket = RecoveryAdmissionTicket {
            ticket_id: random_uuid(),
            operation_id: operation_id.to_owned(),
            payload_digest: payload_digest.to_owned(),
            boot_id: proof.boot_id.clone(),
            instance_incarnation: proof.instance_incarnation.clone(),
            lease_epoch: proof.lease_epoch,
            host_fence_id: fence.host_fence_id.clone(),
            state: RecoveryTicketState::Issued,
            issued_at_millis: now_millis,
            expires_at_millis,
        };
        tx.execute(
            "INSERT INTO admission_tickets (
                 ticket_id, operation_id, instance_id, payload_digest, boot_id,
                 instance_incarnation, lease_epoch, host_fence_id, state,
                 issued_at_millis, expires_at_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                ticket.ticket_id,
                ticket.operation_id,
                instance_id,
                ticket.payload_digest,
                ticket.boot_id,
                ticket.instance_incarnation,
                ticket.lease_epoch as i64,
                ticket.host_fence_id,
                ticket.state.as_str(),
                ticket.issued_at_millis as i64,
                ticket.expires_at_millis as i64,
            ],
        )
        .map_err(super::map_sql_error)?;
        tx.commit().map_err(super::map_sql_error)?;
        Ok(ticket)
    }

    pub fn admission_ticket(
        &self,
        ticket_id: &str,
    ) -> Result<Option<RecoveryAdmissionTicket>, RecoveryStoreError> {
        validate_uuid_v4("ticket_id", ticket_id)?;
        self.conn
            .query_row(
                "SELECT ticket_id, operation_id, payload_digest, boot_id,
                        instance_incarnation, lease_epoch, host_fence_id, state,
                        issued_at_millis, expires_at_millis
                 FROM admission_tickets WHERE ticket_id = ?1",
                [ticket_id],
                row_ticket,
            )
            .optional()
            .map_err(super::map_sql_error)
    }

    pub fn validate_admission_ticket(
        &self,
        proof: &RecoveryLeaseProof,
        fence: &RecoveryHostFence,
        ticket_id: &str,
        now_millis: u64,
    ) -> Result<RecoveryAdmissionTicket, RecoveryStoreError> {
        validate_uuid_v4("ticket_id", ticket_id)?;
        validate_wire(now_millis, "now_millis")?;
        let ticket = self
            .admission_ticket(ticket_id)?
            .ok_or(RecoveryStoreError::AdmissionTicketNotFound)?;
        validate_fence(fence)?;
        if ticket.boot_id != proof.boot_id
            || ticket.instance_incarnation != proof.instance_incarnation
            || ticket.lease_epoch != proof.lease_epoch
            || ticket.host_fence_id != fence.host_fence_id
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        if self.current_host_fence()? != *fence {
            return Err(RecoveryStoreError::StaleLease);
        }
        self.ensure_context(proof, now_millis)?;
        if ticket.expires_at_millis <= now_millis {
            return Err(RecoveryStoreError::AdmissionTicketExpired);
        }
        if !matches!(
            ticket.state,
            RecoveryTicketState::Issued
                | RecoveryTicketState::Admitted
                | RecoveryTicketState::Executing
                | RecoveryTicketState::EffectWitnessRecorded
        ) {
            return Err(RecoveryStoreError::InvalidTransition);
        }
        Ok(ticket)
    }

    pub fn transition_admission_ticket(
        &mut self,
        proof: &RecoveryLeaseProof,
        fence: &RecoveryHostFence,
        ticket_id: &str,
        next_state: RecoveryTicketState,
        now_millis: u64,
    ) -> Result<RecoveryAdmissionTicket, RecoveryStoreError> {
        let ticket = self.validate_admission_ticket(proof, fence, ticket_id, now_millis)?;
        if !valid_ticket_transition(ticket.state, next_state) {
            return Err(RecoveryStoreError::InvalidTransition);
        }
        let changed = self
            .conn
            .execute(
                "UPDATE admission_tickets SET state = ?1
                 WHERE ticket_id = ?2 AND state = ?3 AND expires_at_millis > ?4",
                rusqlite::params![
                    next_state.as_str(),
                    ticket_id,
                    ticket.state.as_str(),
                    now_millis as i64,
                ],
            )
            .map_err(super::map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        Ok(RecoveryAdmissionTicket {
            state: next_state,
            ..ticket
        })
    }

    /// Marks every issued/admitted/executing ticket whose deadline elapsed.
    /// This is an atomic state transition and is safe to call from a renewal
    /// loop that does not perform inference or host I/O.
    pub fn expire_admission_tickets(
        &mut self,
        now_millis: u64,
    ) -> Result<usize, RecoveryStoreError> {
        validate_wire(now_millis, "now_millis")?;
        self.conn
            .execute(
                "UPDATE admission_tickets SET state = 'UNKNOWN'
                 WHERE expires_at_millis <= ?1
                   AND state IN ('ISSUED', 'ADMITTED', 'EXECUTING', 'EFFECT_WITNESS_RECORDED')",
                [now_millis as i64],
            )
            .map_err(super::map_sql_error)
    }
}

fn validate_fence(fence: &RecoveryHostFence) -> Result<(), RecoveryStoreError> {
    validate_uuid_v4("host_fence_id", &fence.host_fence_id)?;
    validate_uuid("deployment_id", &fence.deployment_id)?;
    validate_uuid("instance_id", &fence.instance_id)?;
    validate_uuid_v4("instance_incarnation", &fence.instance_incarnation)?;
    validate_uuid_v4("boot_id", &fence.boot_id)?;
    validate_wire(fence.authority_generation, "authority_generation")?;
    validate_wire(fence.fence_generation, "fence_generation")?;
    validate_wire(fence.created_at_millis, "fence_created_at_millis")
}

fn valid_ticket_transition(from: RecoveryTicketState, to: RecoveryTicketState) -> bool {
    matches!(
        (from, to),
        (RecoveryTicketState::Issued, RecoveryTicketState::Admitted)
            | (RecoveryTicketState::Issued, RecoveryTicketState::Rejected)
            | (RecoveryTicketState::Issued, RecoveryTicketState::Unknown)
            | (
                RecoveryTicketState::Admitted,
                RecoveryTicketState::Executing
            )
            | (RecoveryTicketState::Admitted, RecoveryTicketState::Rejected)
            | (RecoveryTicketState::Admitted, RecoveryTicketState::Unknown)
            | (
                RecoveryTicketState::Executing,
                RecoveryTicketState::EffectWitnessRecorded
            )
            | (RecoveryTicketState::Executing, RecoveryTicketState::Settled)
            | (
                RecoveryTicketState::Executing,
                RecoveryTicketState::Rejected
            )
            | (RecoveryTicketState::Executing, RecoveryTicketState::Unknown)
            | (
                RecoveryTicketState::EffectWitnessRecorded,
                RecoveryTicketState::Settled
            )
            | (
                RecoveryTicketState::EffectWitnessRecorded,
                RecoveryTicketState::Unknown
            )
    )
}

fn row_ticket(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecoveryAdmissionTicket> {
    Ok(RecoveryAdmissionTicket {
        ticket_id: row.get(0)?,
        operation_id: row.get(1)?,
        payload_digest: row.get(2)?,
        boot_id: row.get(3)?,
        instance_incarnation: row.get(4)?,
        lease_epoch: super::row_u64(row, 5)?,
        host_fence_id: row.get(6)?,
        state: RecoveryTicketState::parse(&row.get::<_, String>(7)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        issued_at_millis: super::row_u64(row, 8)?,
        expires_at_millis: super::row_u64(row, 9)?,
    })
}

fn row_ticket_for_operation(
    source: &rusqlite::Connection,
    instance_id: &str,
    operation_id: &str,
) -> Result<Option<RecoveryAdmissionTicket>, RecoveryStoreError> {
    source
        .query_row(
            "SELECT ticket_id, operation_id, payload_digest, boot_id,
                    instance_incarnation, lease_epoch, host_fence_id, state,
                    issued_at_millis, expires_at_millis
             FROM admission_tickets WHERE instance_id = ?1 AND operation_id = ?2",
            rusqlite::params![instance_id, operation_id],
            row_ticket,
        )
        .optional()
        .map_err(super::map_sql_error)
}
