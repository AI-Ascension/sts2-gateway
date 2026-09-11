// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    RecoveryAdmissionTicket, RecoveryHostFence, RecoveryLeaseProof, RecoveryOperationState,
    RecoveryStoreError, RecoveryTicketState, validate_digest, validate_uuid, validate_uuid_v4,
    validate_wire,
};
use super::GatewayRecoveryStore;

pub(super) fn validate_fence(fence: &RecoveryHostFence) -> Result<(), RecoveryStoreError> {
    validate_uuid_v4("host_fence_id", &fence.host_fence_id)?;
    validate_uuid("deployment_id", &fence.deployment_id)?;
    validate_uuid("instance_id", &fence.instance_id)?;
    validate_uuid_v4("instance_incarnation", &fence.instance_incarnation)?;
    validate_uuid_v4("boot_id", &fence.boot_id)?;
    validate_wire(fence.authority_generation, "authority_generation")?;
    validate_wire(fence.fence_generation, "fence_generation")?;
    validate_wire(fence.created_at_millis, "fence_created_at_millis")
}

pub(super) fn valid_ticket_transition(from: RecoveryTicketState, to: RecoveryTicketState) -> bool {
    matches!(
        (from, to),
        (RecoveryTicketState::Issued, RecoveryTicketState::Admitted)
            | (RecoveryTicketState::Issued, RecoveryTicketState::Executing)
            | (
                RecoveryTicketState::Issued,
                RecoveryTicketState::EffectWitnessRecorded
            )
            | (RecoveryTicketState::Issued, RecoveryTicketState::Settled)
            | (RecoveryTicketState::Issued, RecoveryTicketState::Rejected)
            | (RecoveryTicketState::Issued, RecoveryTicketState::Unknown)
            | (
                RecoveryTicketState::Admitted,
                RecoveryTicketState::Executing
            )
            | (
                RecoveryTicketState::Admitted,
                RecoveryTicketState::EffectWitnessRecorded
            )
            | (RecoveryTicketState::Admitted, RecoveryTicketState::Settled)
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

pub(super) fn row_ticket(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecoveryAdmissionTicket> {
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

pub(super) fn row_ticket_for_operation(
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

impl GatewayRecoveryStore {
    /// Imports a host-owned ticket while reconciling an historical operation. Unlike
    /// `record_host_ticket`, this path deliberately does not require the gateway's
    /// current lease or current fence: the ticket's fence belongs to the original
    /// operation and may predate a later authority rotation.
    pub fn record_historical_host_ticket(
        &mut self,
        instance_id: &str,
        ticket: RecoveryAdmissionTicket,
    ) -> Result<RecoveryAdmissionTicket, RecoveryStoreError> {
        validate_uuid("instance_id", instance_id)?;
        validate_uuid_v4("ticket_id", &ticket.ticket_id)?;
        validate_uuid_v4("operation_id", &ticket.operation_id)?;
        validate_digest("payload_digest", &ticket.payload_digest)?;
        validate_uuid_v4("ticket_boot_id", &ticket.boot_id)?;
        validate_uuid_v4("ticket_instance_incarnation", &ticket.instance_incarnation)?;
        validate_uuid_v4("ticket_host_fence_id", &ticket.host_fence_id)?;
        validate_wire(ticket.lease_epoch, "ticket_lease_epoch")?;
        validate_wire(ticket.issued_at_millis, "ticket_issued_at_millis")?;
        validate_wire(ticket.expires_at_millis, "ticket_expires_at_millis")?;
        if ticket.issued_at_millis >= ticket.expires_at_millis {
            return Err(RecoveryStoreError::InvalidInput(
                "host admission ticket deadline is invalid".to_owned(),
            ));
        }
        let tx = self.transaction()?;
        let operation =
            super::operation_helpers::select_operation(&tx, instance_id, &ticket.operation_id)?
                .ok_or(RecoveryStoreError::OperationNotFound)?;
        if operation.payload_digest != ticket.payload_digest
            || operation.boot_id != ticket.boot_id
            || operation.instance_incarnation != ticket.instance_incarnation
            || operation.lease_epoch != ticket.lease_epoch
        {
            return Err(RecoveryStoreError::OperationConflict);
        }
        if let Some(existing) = row_ticket_for_operation(&tx, instance_id, &ticket.operation_id)? {
            if existing.ticket_id != ticket.ticket_id
                || existing.payload_digest != ticket.payload_digest
                || existing.boot_id != ticket.boot_id
                || existing.instance_incarnation != ticket.instance_incarnation
                || existing.lease_epoch != ticket.lease_epoch
                || existing.host_fence_id != ticket.host_fence_id
                || existing.issued_at_millis != ticket.issued_at_millis
                || existing.expires_at_millis != ticket.expires_at_millis
            {
                return Err(RecoveryStoreError::OperationConflict);
            }
            if existing.state != ticket.state
                && !valid_ticket_transition(existing.state, ticket.state)
            {
                return Err(RecoveryStoreError::InvalidTransition);
            }
            tx.execute(
                "UPDATE admission_tickets SET state = ?1 WHERE ticket_id = ?2",
                rusqlite::params![ticket.state.as_str(), ticket.ticket_id],
            )
            .map_err(super::map_sql_error)?;
        } else {
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
        }
        tx.commit().map_err(super::map_sql_error)?;
        Ok(ticket)
    }

    /// Imports the host-owned ticket returned by the recovery control channel. The
    /// ticket identity is retained verbatim so later lookup/reconcile responses
    /// cannot be mistaken for a gateway-generated admission.
    pub fn record_host_ticket(
        &mut self,
        proof: &RecoveryLeaseProof,
        fence: &RecoveryHostFence,
        ticket: RecoveryAdmissionTicket,
        now_millis: u64,
    ) -> Result<RecoveryAdmissionTicket, RecoveryStoreError> {
        validate_uuid_v4("ticket_id", &ticket.ticket_id)?;
        validate_uuid_v4("operation_id", &ticket.operation_id)?;
        validate_digest("payload_digest", &ticket.payload_digest)?;
        validate_uuid_v4("ticket_boot_id", &ticket.boot_id)?;
        validate_uuid_v4("ticket_instance_incarnation", &ticket.instance_incarnation)?;
        validate_uuid_v4("ticket_host_fence_id", &ticket.host_fence_id)?;
        validate_wire(ticket.lease_epoch, "ticket_lease_epoch")?;
        validate_wire(ticket.issued_at_millis, "ticket_issued_at_millis")?;
        validate_wire(ticket.expires_at_millis, "ticket_expires_at_millis")?;
        if ticket.issued_at_millis >= ticket.expires_at_millis {
            return Err(RecoveryStoreError::InvalidInput(
                "host admission ticket deadline is invalid".to_owned(),
            ));
        }
        if ticket.boot_id != proof.boot_id
            || ticket.instance_incarnation != proof.instance_incarnation
            || ticket.lease_epoch != proof.lease_epoch
            || ticket.host_fence_id != fence.host_fence_id
            || fence.boot_id != proof.boot_id
            || fence.instance_incarnation != proof.instance_incarnation
            || fence.authority_generation != proof.authority_generation
            || self.current_host_fence()? != *fence
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        let lease = self.ensure_context(proof, now_millis)?;
        if ticket.expires_at_millis > lease.expires_at_millis {
            return Err(RecoveryStoreError::ContractMismatch(
                "host admission ticket outlives the authenticated lease".to_owned(),
            ));
        }
        let tx = self.transaction()?;
        let operation = super::operation_helpers::select_operation(
            &tx,
            &proof.instance_id,
            &ticket.operation_id,
        )?
        .ok_or(RecoveryStoreError::OperationNotFound)?;
        if operation.payload_digest != ticket.payload_digest
            || operation.boot_id != ticket.boot_id
            || operation.instance_incarnation != ticket.instance_incarnation
            || operation.lease_epoch != ticket.lease_epoch
            || !operation.state.unresolved()
                && !matches!(
                    (operation.state, ticket.state),
                    (
                        RecoveryOperationState::Settled,
                        RecoveryTicketState::Settled
                    ) | (
                        RecoveryOperationState::Rejected,
                        RecoveryTicketState::Rejected
                    )
                )
        {
            return Err(RecoveryStoreError::OperationConflict);
        }
        if let Some(existing) =
            row_ticket_for_operation(&tx, &proof.instance_id, &ticket.operation_id)?
        {
            if existing.ticket_id != ticket.ticket_id
                || existing.payload_digest != ticket.payload_digest
                || existing.boot_id != ticket.boot_id
                || existing.instance_incarnation != ticket.instance_incarnation
                || existing.lease_epoch != ticket.lease_epoch
                || existing.host_fence_id != ticket.host_fence_id
                || existing.issued_at_millis != ticket.issued_at_millis
                || existing.expires_at_millis != ticket.expires_at_millis
            {
                return Err(RecoveryStoreError::OperationConflict);
            }
            if existing.state != ticket.state
                && !valid_ticket_transition(existing.state, ticket.state)
            {
                return Err(RecoveryStoreError::InvalidTransition);
            }
            tx.execute(
                "UPDATE admission_tickets SET state = ?1 WHERE ticket_id = ?2",
                rusqlite::params![ticket.state.as_str(), ticket.ticket_id],
            )
            .map_err(super::map_sql_error)?;
        } else {
            tx.execute(
                "INSERT INTO admission_tickets (
                     ticket_id, operation_id, instance_id, payload_digest, boot_id,
                     instance_incarnation, lease_epoch, host_fence_id, state,
                     issued_at_millis, expires_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    ticket.ticket_id,
                    ticket.operation_id,
                    proof.instance_id,
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
        }
        tx.commit().map_err(super::map_sql_error)?;
        Ok(ticket)
    }
}
