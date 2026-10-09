//! SQLite connection setup and schema migration (user_version based).

use crate::error::CaseError;
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 1;

pub fn open(path: &Path) -> Result<Connection, CaseError> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<(), CaseError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(CaseError::SchemaTooNew(version));
    }
    if version < 1 {
        conn.execute_batch(include_str!("schema.sql"))?;
        conn.execute_batch("PRAGMA user_version = 1;")?;
    }
    Ok(())
}
