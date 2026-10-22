// SPDX-License-Identifier: MIT

use rusqlite::{Connection, params};

use super::provisioning_record::{MAX_ALLOCATIONS, MAX_RECORD_BYTES, decode, validate_set};
use super::provisioning_types::{
    UserDataProvisioningError as Error, UserDataProvisioningRecord as Record,
};

pub(super) fn validate_owner_row(connection: &Connection) -> Result<(), Error> {
    let (total, valid): (i64, i64) = connection
        .query_row(
            "SELECT count(*),coalesce(sum(CASE WHEN singleton=1 AND typeof(token)='blob'
             AND length(token)=16 THEN 1 ELSE 0 END),0) FROM save_profile_coordinator",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_error)?;
    if !(0..=1).contains(&total) || total != valid {
        return Err(Error::PersistenceFailed);
    }
    Ok(())
}

pub(super) fn read_records(connection: &Connection) -> Result<Vec<Record>, Error> {
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM save_profile_user_data_records",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if count > MAX_ALLOCATIONS as i64 {
        return Err(Error::CapacityExceeded);
    }
    let mut statement = connection
        .prepare(
            "SELECT instance_id,operation_id,body,length(body),
             length(CAST(instance_id AS BLOB)),length(CAST(operation_id AS BLOB)),typeof(body)
             FROM save_profile_user_data_records ORDER BY instance_id,operation_id",
        )
        .map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        let length: i64 = row.get(3).map_err(db_error)?;
        let instance_length: i64 = row.get(4).map_err(db_error)?;
        let operation_length: i64 = row.get(5).map_err(db_error)?;
        let storage_type: String = row.get(6).map_err(db_error)?;
        if !(1..=MAX_RECORD_BYTES as i64).contains(&length)
            || !(1..=128).contains(&instance_length)
            || !(1..=128).contains(&operation_length)
            || storage_type != "blob"
        {
            return Err(Error::PersistenceFailed);
        }
        let bytes: Vec<u8> = row.get(2).map_err(db_error)?;
        let record = decode(&bytes)?;
        let instance: String = row.get(0).map_err(db_error)?;
        let operation: String = row.get(1).map_err(db_error)?;
        if instance != record.context.instance_id || operation != record.operation_id {
            return Err(Error::PersistenceFailed);
        }
        records.push(record);
    }
    validate_set(&records, MAX_ALLOCATIONS)?;
    Ok(records)
}

pub(super) fn verify_write(
    connection: &Connection,
    record: &Record,
    bytes: &[u8],
) -> Result<(), Error> {
    let actual: Vec<u8> = connection
        .query_row(
            "SELECT body FROM save_profile_user_data_records WHERE instance_id=?1 AND operation_id=?2",
            params![record.context.instance_id, record.operation_id],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if actual != bytes {
        return Err(Error::PersistenceFailed);
    }
    Ok(())
}

pub(super) fn db_error(_: rusqlite::Error) -> Error {
    Error::PersistenceFailed
}
