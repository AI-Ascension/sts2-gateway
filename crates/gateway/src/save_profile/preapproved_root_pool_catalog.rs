// SPDX-License-Identifier: MIT

//! Test-only, crash-safe identity seal for the entire finite approved root set.
//! A durable root claim is not evidence that *unclaimed* roots were unchanged.
//! This module supplies no native-root, host-save, process, or idle authority.

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::preapproved_root_pool::{ApprovedRoot, PoolError};

/// Binds a journal to the complete exact catalog, including unclaimed roots.
/// The snapshot is committed before any subsequent candidate claim can succeed.
/// Legacy journals with existing claims but no snapshot refuse closed.
pub(super) fn seal_exact_catalog(
    connection: &mut Connection,
    catalog: &BTreeMap<String, ApprovedRoot>,
) -> Result<(), PoolError> {
    let entries = catalog
        .iter()
        .map(|(key, root)| {
            (
                key.as_str(),
                root.profile.value(),
                root.namespace,
                root.image_digest.as_str(),
            )
        })
        .collect::<Vec<_>>();
    let canonical = serde_json::to_string(&entries).map_err(|_| PoolError::InvalidCatalog)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| PoolError::Storage)?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS approved_root_catalog (
                 singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
                 exact_roots TEXT NOT NULL
             );",
        )
        .map_err(|_| PoolError::Storage)?;
    let previous: Option<String> = transaction
        .query_row(
            "SELECT exact_roots FROM approved_root_catalog WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| PoolError::Storage)?;
    match previous {
        Some(snapshot) if snapshot == canonical => {}
        Some(_) => return Err(PoolError::InvalidCatalog),
        None => {
            let prior_claims: i64 = transaction
                .query_row("SELECT COUNT(*) FROM root_claims", [], |row| row.get(0))
                .map_err(|_| PoolError::Storage)?;
            if prior_claims != 0 {
                return Err(PoolError::InvalidCatalog);
            }
            transaction
                .execute(
                    "INSERT INTO approved_root_catalog (singleton, exact_roots)
                     VALUES (1, ?1)",
                    params![canonical],
                )
                .map_err(|_| PoolError::Storage)?;
        }
    }
    transaction.commit().map_err(|_| PoolError::Storage)?;
    Ok(())
}
