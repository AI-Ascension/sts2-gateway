// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    RecoveryOperation, RecoveryOperationState, RecoveryStoreError, RecoveryUncertaintyReason,
};

pub(super) fn select_operation(
    source: &rusqlite::Connection,
    instance_id: &str,
    operation_id: &str,
) -> Result<Option<RecoveryOperation>, RecoveryStoreError> {
    let active = source
        .query_row(
            &format!(
                "SELECT {} FROM operations WHERE instance_id = ?1 AND operation_id = ?2",
                super::sql::OPERATION_COLUMNS
            ),
            rusqlite::params![instance_id, operation_id],
            row_operation,
        )
        .optional()
        .map_err(super::map_sql_error)?;
    if active.is_some() {
        return Ok(active);
    }
    source
        .query_row(
            &format!(
                "SELECT {} FROM operation_archive
                 WHERE instance_id = ?1 AND operation_id = ?2",
                super::sql::ARCHIVE_COLUMNS
            ),
            rusqlite::params![instance_id, operation_id],
            row_operation,
        )
        .optional()
        .map_err(super::map_sql_error)
}

fn row_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecoveryOperation> {
    let witness = row
        .get::<_, Option<Vec<u8>>>(17)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()?;
    let uncertainty = row
        .get::<_, Option<String>>(18)?
        .map(|value| parse_uncertainty(&value).ok_or(rusqlite::Error::InvalidQuery))
        .transpose()?;
    Ok(RecoveryOperation {
        instance_id: row.get(0)?,
        operation_id: row.get(1)?,
        deployment_id: row.get(2)?,
        instance_incarnation: row.get(3)?,
        boot_id: row.get(4)?,
        authority_generation: super::row_u64(row, 5)?,
        lease_id: row.get(6)?,
        lease_epoch: super::row_u64(row, 7)?,
        state: RecoveryOperationState::parse(&row.get::<_, String>(8)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        payload_digest: row.get(9)?,
        schema_digest: row.get(10)?,
        canonical_json: row.get(11)?,
        expected_state_id: row.get(12)?,
        expected_generation: super::row_u64(row, 13)?,
        catalog_digest: row.get(14)?,
        response_status: row.get(15)?,
        response_body: row.get(16)?,
        witness,
        uncertainty_reason: uncertainty,
        created_at_millis: super::row_u64(row, 19)?,
        updated_at_millis: super::row_u64(row, 20)?,
    })
}

pub(super) fn insert_operation(
    tx: &rusqlite::Transaction<'_>,
    operation: &RecoveryOperation,
) -> Result<(), RecoveryStoreError> {
    tx.execute(
        &format!(
            "INSERT INTO operations ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
            super::sql::OPERATION_COLUMNS
        ),
        operation_params(operation),
    )
    .map(|_| ())
    .map_err(super::map_sql_error)
}

pub(super) fn update_operation(
    tx: &rusqlite::Transaction<'_>,
    operation: &RecoveryOperation,
) -> Result<(), RecoveryStoreError> {
    let changed = tx
        .execute(
            "UPDATE operations SET state = ?1, payload_digest = ?2, schema_digest = ?3,
             canonical_json = ?4, expected_state_id = ?5, expected_generation = ?6,
             catalog_digest = ?7, response_status = ?8, response_body = ?9,
             witness_json = ?10, uncertainty_reason = ?11, updated_at_millis = ?12
         WHERE instance_id = ?13 AND operation_id = ?14",
            rusqlite::params![
                operation.state.as_str(),
                operation.payload_digest,
                operation.schema_digest,
                operation.canonical_json,
                operation.expected_state_id,
                operation.expected_generation as i64,
                operation.catalog_digest,
                operation.response_status,
                operation.response_body,
                witness_bytes(operation),
                operation.uncertainty_reason.map(|reason| reason.as_str()),
                operation.updated_at_millis as i64,
                operation.instance_id,
                operation.operation_id,
            ],
        )
        .map_err(super::map_sql_error)?;
    if changed != 1 {
        return Err(RecoveryStoreError::OperationNotFound);
    }
    Ok(())
}

#[allow(clippy::useless_conversion)]
fn operation_params(
    operation: &RecoveryOperation,
) -> rusqlite::ParamsFromIter<std::vec::IntoIter<rusqlite::types::Value>> {
    rusqlite::params_from_iter(
        vec![
            rusqlite::types::Value::Text(operation.instance_id.clone()),
            rusqlite::types::Value::Text(operation.operation_id.clone()),
            rusqlite::types::Value::Text(operation.deployment_id.clone()),
            rusqlite::types::Value::Text(operation.instance_incarnation.clone()),
            rusqlite::types::Value::Text(operation.boot_id.clone()),
            rusqlite::types::Value::Integer(operation.authority_generation as i64),
            rusqlite::types::Value::Text(operation.lease_id.clone()),
            rusqlite::types::Value::Integer(operation.lease_epoch as i64),
            rusqlite::types::Value::Text(operation.state.as_str().to_owned()),
            rusqlite::types::Value::Text(operation.payload_digest.clone()),
            rusqlite::types::Value::Text(operation.schema_digest.clone()),
            rusqlite::types::Value::Blob(operation.canonical_json.clone()),
            rusqlite::types::Value::Text(operation.expected_state_id.clone()),
            rusqlite::types::Value::Integer(operation.expected_generation as i64),
            rusqlite::types::Value::Text(operation.catalog_digest.clone()),
            operation
                .response_status
                .map_or(rusqlite::types::Value::Null, |value| {
                    rusqlite::types::Value::Integer(i64::from(value))
                }),
            operation
                .response_body
                .clone()
                .map_or(rusqlite::types::Value::Null, rusqlite::types::Value::Blob),
            witness_bytes(operation)
                .map_or(rusqlite::types::Value::Null, rusqlite::types::Value::Blob),
            operation
                .uncertainty_reason
                .map_or(rusqlite::types::Value::Null, |value| {
                    rusqlite::types::Value::Text(value.as_str().to_owned())
                }),
            rusqlite::types::Value::Integer(operation.created_at_millis as i64),
            rusqlite::types::Value::Integer(operation.updated_at_millis as i64),
        ]
        .into_iter(),
    )
}

fn witness_bytes(operation: &RecoveryOperation) -> Option<Vec<u8>> {
    operation
        .witness
        .as_ref()
        .and_then(|witness| serde_json::to_vec(witness).ok())
}

fn parse_uncertainty(value: &str) -> Option<RecoveryUncertaintyReason> {
    Some(match value {
        "transport_lost" => RecoveryUncertaintyReason::TransportLost,
        "timeout" => RecoveryUncertaintyReason::Timeout,
        "gateway_crash" => RecoveryUncertaintyReason::GatewayCrash,
        "host_crash" => RecoveryUncertaintyReason::HostCrash,
        "receipt_missing" => RecoveryUncertaintyReason::ReceiptMissing,
        "authority_rotated" => RecoveryUncertaintyReason::AuthorityRotated,
        _ => return None,
    })
}
