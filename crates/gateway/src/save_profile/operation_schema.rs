// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::ledger_types::SaveProfileLedgerError as Error;

const OWNER: &str = "CREATE TABLE save_profile_operation_owner (
               singleton INTEGER PRIMARY KEY CHECK(singleton=1), token BLOB NOT NULL)";
const RECORDS: &str = "CREATE TABLE save_profile_operation_records (
               sequence INTEGER PRIMARY KEY,
               instance_id TEXT NOT NULL, operation_id TEXT NOT NULL, body BLOB NOT NULL,
               UNIQUE(instance_id, operation_id))";

/// A dedicated private journal has exactly these tables and its one constraint index.
/// Reject triggers/views/extra indexes before any write or recovered authority is trusted.
pub(super) fn verify(connection: &Connection) -> Result<(), Error> {
    let mut statement = connection
        .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY name")
        .map_err(|_| Error::PersistenceFailed)?;
    let mut rows = statement.query([]).map_err(|_| Error::PersistenceFailed)?;
    for (kind, name, sql) in [
        ("table", "save_profile_operation_owner", Some(OWNER)),
        ("table", "save_profile_operation_records", Some(RECORDS)),
        (
            "index",
            "sqlite_autoindex_save_profile_operation_records_1",
            None,
        ),
    ] {
        let Some(row) = rows.next().map_err(|_| Error::PersistenceFailed)? else {
            return Err(Error::PersistenceFailed);
        };
        let actual: (String, String, Option<String>) = (
            row.get(0).map_err(|_| Error::PersistenceFailed)?,
            row.get(1).map_err(|_| Error::PersistenceFailed)?,
            row.get(2).map_err(|_| Error::PersistenceFailed)?,
        );
        if actual.0 != kind || actual.1 != name || actual.2.as_deref() != sql {
            return Err(Error::PersistenceFailed);
        }
    }
    if rows.next().map_err(|_| Error::PersistenceFailed)?.is_some() {
        return Err(Error::PersistenceFailed);
    }
    Ok(())
}

pub(super) fn initialize(connection: &Connection) -> Result<(), Error> {
    let count: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
        .map_err(|_| Error::PersistenceFailed)?;
    if count == 0 {
        connection
            .execute_batch(&format!("{OWNER};{RECORDS};"))
            .map_err(|_| Error::PersistenceFailed)?;
    }
    verify(connection)
}
