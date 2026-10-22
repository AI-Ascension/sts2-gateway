// SPDX-License-Identifier: MIT

use super::*;
use rusqlite::{Connection, params};

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        Self(temp_database())
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        remove_database(&self.0);
    }
}

#[test]
fn duplicate_allocation_and_changed_ownership_are_refused_without_changes() -> Result<(), String> {
    let mut store =
        SqliteUserDataRecordStore::open_in_memory().map_err(|error| format!("{error:?}"))?;
    let original = record(1, "op-1")?;
    store
        .insert(original.clone())
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        store.insert(record(1, "op-2")?),
        Err(UserDataProvisioningError::OperationConflict)
    );
    for case in 0..4 {
        let mut changed = original.clone();
        changed.status = UserDataProvisioningStatus::Created;
        match case {
            0 => changed.context.lease_id = "foreign-lease".to_owned(),
            1 => changed.context.caller_id = "foreign-caller".to_owned(),
            2 => {
                changed.descriptor.identity = UserDataIdentity::new(2);
                changed.launch_profile.user_data = UserDataIdentity::new(2);
            }
            _ => changed.context.correlation_id = "foreign-correlation".to_owned(),
        }
        assert_eq!(
            store.update(changed),
            Err(UserDataProvisioningError::OperationConflict)
        );
    }
    assert_eq!(
        store.list().map_err(|error| format!("{error:?}"))?,
        vec![original]
    );
    Ok(())
}

#[test]
fn malformed_oversized_misbound_and_duplicate_rows_refuse_reopen() -> Result<(), String> {
    for case in 0..7 {
        let database = Database::new();
        let mut store =
            SqliteUserDataRecordStore::open(&database.0).map_err(|error| format!("{error:?}"))?;
        store
            .insert(record(1, "op-1")?)
            .map_err(|error| format!("{error:?}"))?;
        let connection = Connection::open(&database.0).map_err(|error| error.to_string())?;
        match case {
            0 => {
                connection
                    .execute(
                        "UPDATE save_profile_user_data_records SET body=zeroblob(8193)",
                        [],
                    )
                    .map_err(|error| error.to_string())?;
            }
            1 => {
                connection
                    .execute(
                        "UPDATE save_profile_user_data_records SET operation_id='foreign'",
                        [],
                    )
                    .map_err(|error| error.to_string())?;
            }
            2 => {
                let bytes =
                    serde_json::to_vec(&record(1, "op-2")?).map_err(|error| error.to_string())?;
                connection
                    .execute(
                        "INSERT INTO save_profile_user_data_records VALUES(?1,'op-2',?2)",
                        params![INSTANCE, bytes],
                    )
                    .map_err(|error| error.to_string())?;
            }
            3 => {
                let mut malformed = record(1, "op-1")?;
                malformed.descriptor.provenance.instance_id = "foreign".to_owned();
                let bytes = serde_json::to_vec(&malformed).map_err(|error| error.to_string())?;
                connection
                    .execute("UPDATE save_profile_user_data_records SET body=?1", [bytes])
                    .map_err(|error| error.to_string())?;
            }
            4 => {
                let mut value =
                    serde_json::to_value(record(1, "op-1")?).map_err(|error| error.to_string())?;
                value["extra"] = serde_json::json!(true);
                connection
                    .execute(
                        "UPDATE save_profile_user_data_records SET body=?1",
                        [serde_json::to_vec(&value).map_err(|error| error.to_string())?],
                    )
                    .map_err(|error| error.to_string())?;
            }
            5 => {
                connection.execute_batch("CREATE TRIGGER suppress_update BEFORE UPDATE ON save_profile_user_data_records BEGIN SELECT RAISE(IGNORE); END;")
                    .map_err(|error| error.to_string())?;
            }
            _ => {
                connection
                    .execute(
                        "UPDATE save_profile_coordinator SET token=zeroblob(8193)",
                        [],
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        assert!(store.list().is_err(), "case {case}");
        drop(store);
        assert!(
            SqliteUserDataRecordStore::open(&database.0).is_err(),
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn unknown_database_and_ephemeral_path_are_not_adopted() -> Result<(), String> {
    assert!(SqliteUserDataRecordStore::open(":memory:").is_err());
    let database = Database::new();
    let connection = Connection::open(&database.0).map_err(|error| error.to_string())?;
    connection
        .execute_batch("CREATE TABLE valued(value TEXT); INSERT INTO valued VALUES('preserve');")
        .map_err(|error| error.to_string())?;
    assert!(SqliteUserDataRecordStore::open(&database.0).is_err());
    let value: String = connection
        .query_row("SELECT value FROM valued", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    assert_eq!(value, "preserve");
    let count: u32 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn restored_records_cannot_exceed_budget_or_share_an_allocation() -> Result<(), String> {
    for (identity, capacity) in [(1, 4), (5, 4)] {
        let mut store = sts2_gateway::InMemoryUserDataRecordStore::default();
        store
            .insert(record(1, "op-1")?)
            .map_err(|error| format!("{error:?}"))?;
        store
            .insert(record(identity, "op-2")?)
            .map_err(|error| format!("{error:?}"))?;
        assert!(
            UserDataProvisioner::new(
                capacity,
                InMemoryUserDataPort::default(),
                store,
                InMemoryLaunchProfileBindingPort::default()
            )
            .is_err()
        );
    }
    Ok(())
}
