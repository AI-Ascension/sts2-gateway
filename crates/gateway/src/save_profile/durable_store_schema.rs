// SPDX-License-Identifier: MIT

use rusqlite::Connection;

use super::durable_store_query::db_error;
use super::provisioning_types::UserDataProvisioningError as Error;

pub(super) fn initialize(connection: &Connection) -> Result<(), Error> {
    if objects(connection)?.is_empty() {
        connection
            .execute_batch(
                "CREATE TABLE save_profile_user_data_records (
                   instance_id TEXT NOT NULL,
                   operation_id TEXT NOT NULL,
                   body BLOB NOT NULL,
                   PRIMARY KEY (instance_id, operation_id)
                 );
                 CREATE TABLE save_profile_coordinator (
                   singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                   token BLOB NOT NULL
                 );",
            )
            .map_err(db_error)?;
    }
    verify(connection)
}

pub(super) fn verify_or_empty(connection: &Connection) -> Result<(), Error> {
    if objects(connection)?.is_empty() {
        return Ok(());
    }
    verify(connection)
}

pub(super) fn verify(connection: &Connection) -> Result<(), Error> {
    if objects(connection)?
        != [
            ("table".to_owned(), "save_profile_coordinator".to_owned()),
            (
                "table".to_owned(),
                "save_profile_user_data_records".to_owned(),
            ),
        ]
    {
        return Err(Error::PersistenceFailed);
    }
    let tables = [
        (
            "save_profile_user_data_records",
            vec![
                ("instance_id", "TEXT", 1, 1),
                ("operation_id", "TEXT", 1, 2),
                ("body", "BLOB", 1, 0),
            ],
        ),
        (
            "save_profile_coordinator",
            vec![("singleton", "INTEGER", 0, 1), ("token", "BLOB", 1, 0)],
        ),
    ];
    for (table, expected) in tables {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(db_error)?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u8>(3)?,
                    row.get::<_, u8>(5)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(name, kind, required, key)| (name.to_owned(), kind.to_owned(), required, key))
            .collect();
        if columns != expected {
            return Err(Error::PersistenceFailed);
        }
    }
    Ok(())
}

fn objects(connection: &Connection) -> Result<Vec<(String, String)>, Error> {
    let mut statement = connection
        .prepare(
            "SELECT substr(type,1,16),substr(name,1,64) FROM sqlite_master
             WHERE substr(name,1,7) <> 'sqlite_' ORDER BY name LIMIT 3",
        )
        .map_err(db_error)?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}
