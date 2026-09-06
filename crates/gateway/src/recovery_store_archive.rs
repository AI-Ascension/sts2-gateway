// SPDX-License-Identifier: MIT

use super::super::recovery_types::{
    RECOVERY_TOMBSTONE_RETENTION_MILLIS, RecoveryStoreError, validate_wire,
};
use super::GatewayRecoveryStore;

impl GatewayRecoveryStore {
    /// Moves terminal records older than the documented tombstone horizon to
    /// the archive in one transaction. The archive retains the complete
    /// operation record, so a retry still receives a duplicate result and a
    /// conflicting payload still receives a conflict.
    pub fn archive_resolved_before(
        &mut self,
        cutoff_millis: u64,
        now_millis: u64,
    ) -> Result<usize, RecoveryStoreError> {
        validate_wire(cutoff_millis, "archive_cutoff_millis")?;
        validate_wire(now_millis, "now_millis")?;
        let horizon_cutoff = now_millis
            .checked_sub(RECOVERY_TOMBSTONE_RETENTION_MILLIS)
            .ok_or(RecoveryStoreError::InvalidInput(
                "archive horizon has not elapsed".to_owned(),
            ))?;
        if cutoff_millis > horizon_cutoff {
            return Err(RecoveryStoreError::InvalidInput(
                "resolved operations must remain retained through the tombstone horizon".to_owned(),
            ));
        }
        let tx = self.transaction()?;
        let moved: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM operations
                 WHERE state IN ('SETTLED', 'REJECTED', 'RECONCILED')
                   AND updated_at_millis <= ?1",
                [cutoff_millis as i64],
                |row| row.get(0),
            )
            .map_err(super::map_sql_error)?;
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO operation_archive ({}, archived_at_millis)
                 SELECT {}, ?1 FROM operations
                 WHERE state IN ('SETTLED', 'REJECTED', 'RECONCILED')
                   AND updated_at_millis <= ?2",
                super::sql::ARCHIVE_COLUMNS,
                super::sql::OPERATION_COLUMNS,
            ),
            rusqlite::params![now_millis as i64, cutoff_millis as i64],
        )
        .map_err(super::map_sql_error)?;
        tx.execute(
            "DELETE FROM operations
             WHERE state IN ('SETTLED', 'REJECTED', 'RECONCILED')
               AND updated_at_millis <= ?1",
            [cutoff_millis as i64],
        )
        .map_err(super::map_sql_error)?;
        tx.commit().map_err(super::map_sql_error)?;
        usize::try_from(moved)
            .map_err(|_| RecoveryStoreError::Corrupt("archive count overflow".to_owned()))
    }

    /// Expires archived tombstones only after their complete retention
    /// horizon. This is the sole operation-id reuse window.
    pub fn purge_expired_archives(&mut self, now_millis: u64) -> Result<usize, RecoveryStoreError> {
        validate_wire(now_millis, "now_millis")?;
        let cutoff = now_millis.saturating_sub(RECOVERY_TOMBSTONE_RETENTION_MILLIS);
        self.conn
            .execute(
                "DELETE FROM operation_archive WHERE archived_at_millis <= ?1",
                [cutoff as i64],
            )
            .map_err(super::map_sql_error)
    }

    pub fn archived_count(&self) -> Result<usize, RecoveryStoreError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM operation_archive", [], |row| {
                row.get(0)
            })
            .map_err(super::map_sql_error)?;
        usize::try_from(count)
            .map_err(|_| RecoveryStoreError::Corrupt("archive count overflow".to_owned()))
    }
}
