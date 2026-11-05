// SPDX-License-Identifier: MIT

//! Strict positive signed SQLite values for test-only candidate root claims.
//! Persisted owner/lease generations must never wrap into apparently valid u64s.
pub(super) fn positive_persisted_u64(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<u64> {
    let value: i64 = row.get(column)?;
    if value <= 0 {
        return Err(rusqlite::Error::IntegralValueOutOfRange(column, value));
    }
    Ok(value as u64)
}
