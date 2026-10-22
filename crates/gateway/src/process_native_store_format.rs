// SPDX-License-Identifier: MIT

use super::{NativeProcessError, db_error};
use rusqlite::Connection;

pub(super) fn initialize(connection: &Connection) -> Result<String, NativeProcessError> {
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .map_err(db_error)?
        .query_map([], |row| row.get(0))
        .map_err(db_error)?
        .collect::<Result<_, _>>()
        .map_err(db_error)?;
    let unsupported: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type IN ('view','trigger')",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if unsupported != 0 {
        return Err(NativeProcessError::Registry);
    }
    if !tables.is_empty()
        && tables
            != [
                "process_generations",
                "process_registry_format",
                "sqlite_sequence",
            ]
    {
        return Err(NativeProcessError::Registry);
    }
    if tables.is_empty() {
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE process_registry_format (version INTEGER NOT NULL, identity TEXT NOT NULL);
                CREATE TABLE process_generations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    generation_key TEXT UNIQUE NOT NULL,
                    record BLOB NOT NULL CHECK(length(record) <= 16384));
                ",
            )
            .map_err(db_error)?;
        connection
            .execute(
                "INSERT INTO process_registry_format(version,identity) VALUES(1,?1)",
                [uuid::Uuid::new_v4().to_string()],
            )
            .map_err(db_error)?;
        connection.execute_batch("COMMIT;").map_err(db_error)?;
    }
    let versions: Vec<(i64, String)> = connection
        .prepare("SELECT version,identity FROM process_registry_format")
        .map_err(db_error)?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(db_error)?
        .collect::<Result<_, _>>()
        .map_err(db_error)?;
    if versions.len() != 1 || versions[0].0 != 1 || uuid::Uuid::parse_str(&versions[0].1).is_err() {
        return Err(NativeProcessError::Registry);
    }
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(db_error)?;
    Ok(versions[0].1.clone())
}
