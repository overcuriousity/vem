//! SQLite connection setup and schema migration (user_version based).

use crate::error::CaseError;
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 1;

pub fn open(path: &Path) -> Result<Connection, CaseError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<(), CaseError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(CaseError::SchemaTooNew(version));
    }
    // Each step runs in one transaction with its `user_version` bump (DDL and user_version are
    // transactional in SQLite), so a crash cannot leave a half-created schema. Later versions add
    // `if version < N { BEGIN; <changes>; PRAGMA user_version = N; COMMIT; }` steps here, in order.
    if version < 1 {
        conn.execute_batch(&format!(
            "BEGIN;\n{}\nPRAGMA user_version = 1;\nCOMMIT;",
            include_str!("schema.sql")
        ))?;
    }
    Ok(())
}
