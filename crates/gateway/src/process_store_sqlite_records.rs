// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::{
    InstanceId, LifecycleOperation, LifecycleOwnership, LifecycleRecordKey, LifecycleRecordStore,
    LifecycleStoreError, SqliteLifecycleStore, admission,
};

impl LifecycleRecordStore for SqliteLifecycleStore {
    fn get(
        &self,
        key: LifecycleRecordKey,
    ) -> Result<Option<LifecycleOperation>, LifecycleStoreError> {
        admission::ensure_token_read(self)?;
        let instance_id = Self::sql_id(key.instance_id().value())?;
        let operation_id = Self::sql_id(key.operation_id().value())?;
        self.connection
            .query_row(
                "SELECT body FROM lifecycle_operations WHERE instance_id = ?1 AND operation_id = ?2",
                rusqlite::params![instance_id, operation_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|_| LifecycleStoreError::Database)?
            .map(|bytes| Self::decode(&bytes))
            .transpose()
    }

    fn insert(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        let bytes = Self::encode(&operation)?;
        let instance_id = Self::sql_id(operation.instance_id().value())?;
        let operation_id = Self::sql_id(operation.operation_id().value())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| LifecycleStoreError::Database)?;
        admission::ensure_token(&transaction, &self.coordinator_token)?;
        if !admission::can_insert_operation(&transaction, &operation, instance_id)? {
            return Err(LifecycleStoreError::Conflict);
        }
        transaction
            .execute(
                "INSERT INTO lifecycle_operations (instance_id, operation_id, body)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![instance_id, operation_id, bytes],
            )
            .map(|_| ())
            .map_err(Self::insert_error)?;
        transaction
            .commit()
            .map_err(|_| LifecycleStoreError::Database)
    }

    fn update(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        let bytes = Self::encode(&operation)?;
        let instance_id = Self::sql_id(operation.instance_id().value())?;
        let operation_id = Self::sql_id(operation.operation_id().value())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| LifecycleStoreError::Database)?;
        admission::ensure_token(&transaction, &self.coordinator_token)?;
        let changed = transaction
            .execute(
                "UPDATE lifecycle_operations SET body = ?1
                 WHERE instance_id = ?2 AND operation_id = ?3",
                rusqlite::params![bytes, instance_id, operation_id],
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        if changed != 1 {
            return Err(LifecycleStoreError::NotFound);
        }
        transaction
            .commit()
            .map_err(|_| LifecycleStoreError::Database)
    }

    fn list(&self) -> Result<Vec<LifecycleOperation>, LifecycleStoreError> {
        admission::ensure_token_read(self)?;
        let mut statement = self
            .connection
            .prepare("SELECT body FROM lifecycle_operations ORDER BY instance_id, operation_id")
            .map_err(|_| LifecycleStoreError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|_| LifecycleStoreError::Database)?;
        rows.map(|row| {
            row.map_err(|_| LifecycleStoreError::Database)
                .and_then(|bytes| Self::decode(&bytes))
        })
        .collect()
    }

    fn count(&self) -> Result<usize, LifecycleStoreError> {
        admission::ensure_token_read(self)?;
        self.connection
            .query_row("SELECT COUNT(*) FROM lifecycle_operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|_| LifecycleStoreError::Database)
            .and_then(|count| {
                usize::try_from(count).map_err(|_| LifecycleStoreError::Serialization)
            })
    }

    fn list_ownership(&self) -> Result<Vec<LifecycleOwnership>, LifecycleStoreError> {
        admission::ensure_token_read(self)?;
        let mut statement = self
            .connection
            .prepare("SELECT body FROM lifecycle_ownership ORDER BY instance_id")
            .map_err(|_| LifecycleStoreError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|_| LifecycleStoreError::Database)?;
        rows.map(|row| {
            row.map_err(|_| LifecycleStoreError::Database)
                .and_then(|bytes| Self::decode_ownership(&bytes))
        })
        .collect()
    }

    fn set_ownership(&mut self, ownership: LifecycleOwnership) -> Result<(), LifecycleStoreError> {
        let bytes = Self::encode_ownership(&ownership)?;
        let instance_id = Self::sql_id(ownership.instance_id().value())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| LifecycleStoreError::Database)?;
        admission::ensure_token(&transaction, &self.coordinator_token)?;
        let current = transaction
            .query_row(
                "SELECT body FROM lifecycle_ownership WHERE instance_id = ?1",
                rusqlite::params![instance_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|_| LifecycleStoreError::Database)?
            .map(|body| Self::decode_ownership(&body))
            .transpose()?;
        admission::ownership_update_allowed(&transaction, &ownership, current.as_ref())?;
        transaction
            .execute(
                "INSERT INTO lifecycle_ownership (instance_id, body)
                 VALUES (?1, ?2)
                 ON CONFLICT(instance_id) DO UPDATE SET body = excluded.body",
                rusqlite::params![instance_id, bytes],
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        transaction
            .commit()
            .map_err(|_| LifecycleStoreError::Database)
    }

    fn clear_ownership_if(
        &mut self,
        ownership: &LifecycleOwnership,
    ) -> Result<(), LifecycleStoreError> {
        let instance_id = Self::sql_id(ownership.instance_id().value())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| LifecycleStoreError::Database)?;
        admission::ensure_token(&transaction, &self.coordinator_token)?;
        let current = transaction
            .query_row(
                "SELECT body FROM lifecycle_ownership WHERE instance_id = ?1",
                rusqlite::params![instance_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|_| LifecycleStoreError::Database)?
            .map(|body| Self::decode_ownership(&body))
            .transpose()?;
        match current {
            None => {}
            Some(current) if current == *ownership => {
                transaction
                    .execute(
                        "DELETE FROM lifecycle_ownership WHERE instance_id = ?1",
                        rusqlite::params![instance_id],
                    )
                    .map_err(|_| LifecycleStoreError::Database)?;
            }
            Some(_) => return Err(LifecycleStoreError::Conflict),
        }
        transaction
            .commit()
            .map_err(|_| LifecycleStoreError::Database)
    }

    fn clear_ownership(&mut self, instance_id: InstanceId) -> Result<(), LifecycleStoreError> {
        let instance_id = Self::sql_id(instance_id.value())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|_| LifecycleStoreError::Database)?;
        admission::ensure_token(&transaction, &self.coordinator_token)?;
        transaction
            .execute(
                "DELETE FROM lifecycle_ownership WHERE instance_id = ?1",
                rusqlite::params![instance_id],
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        transaction
            .commit()
            .map_err(|_| LifecycleStoreError::Database)
    }
}
