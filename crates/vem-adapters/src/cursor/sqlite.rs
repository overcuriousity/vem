//! Opening Cursor's SQLite files without touching the evidence.
//!
//! An evidence db is opened only through [`open_immutable`] (`mode=ro&immutable=1`: no lock, no `-shm`,
//! no `-wal` is created or read). When the manifest lists a non-empty `<db>-wal`, [`open_for_parse`]
//! instead reads a *WAL-applied view*: a private copy of the db and its WAL in a temp dir, opened there.
//! Rows are not tagged by whether they live in the WAL; `wal_applied` only says the view had a WAL.
//! Every connection runs with `trusted_schema = OFF` and `SQLITE_DBCONFIG_DEFENSIVE`, and callers check
//! tables with [`has_table`], so a view or virtual table under an expected name is absent.

use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use vem_core::adapter::{FileContext, ParseError};
use vem_core::hash::{copy_and_hash, sha256_file};
use vem_core::model::{AnomalyDraft, AnomalyKind, Severity};
use vem_core::sink::{ParseSink, RootFileError};

/// The largest `-wal` file applied; a larger one is reported and the db is read without it.
pub const WAL_CAP: u64 = 256 * 1024 * 1024;
/// The largest single value read from a db; checked with `length(value)` before fetching.
pub const RECORD_CAP: u64 = 64 * 1024 * 1024;

/// The immutable read-only URI of `path`: every byte outside `[A-Za-z0-9/._~-]` is percent-encoded, so
/// `?`, `#`, `%` and non-UTF-8 bytes in a path cannot change the URI's meaning.
pub fn sqlite_uri(path: &Path) -> String {
    let mut out = String::from("file:");
    for &b in path_bytes(path).iter() {
        if b.is_ascii_alphanumeric() || b"/._~-".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.push_str("?mode=ro&immutable=1");
    out
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

fn invalid(e: impl std::fmt::Display) -> ParseError {
    ParseError::Invalid(format!("sqlite: {e}"))
}

/// Applies the connection hardening and checks that the file reads as a database.
fn harden(conn: Connection) -> Result<Connection, ParseError> {
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(invalid)?;
    conn.execute_batch("PRAGMA trusted_schema = OFF;")
        .map_err(invalid)?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(invalid)?;
    Ok(conn)
}

/// Opens an evidence db read-only and immutable. Creates nothing next to it.
pub fn open_immutable(path: &Path) -> Result<Connection, ParseError> {
    let conn = Connection::open_with_flags(
        sqlite_uri(path),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(invalid)?;
    harden(conn)
}

/// A db opened for parsing. `wal_applied`: read from a WAL-applied view of the db (its private copy
/// with the `-wal` file applied), not "these rows live in the WAL".
pub struct DbView {
    pub conn: Connection,
    pub wal_applied: bool,
    /// Holds the private copy; declared after `conn` so the connection closes first.
    _tmp: Option<TempDir>,
}

const CHANGED: &str = "evidence changed since inventory";

fn wal_anomaly(ctx: &FileContext<'_>, severity: Severity, message: String) -> AnomalyDraft {
    AnomalyDraft {
        kind: AnomalyKind::WalApplied,
        severity,
        source_file: Some(ctx.handle),
        session: None,
        byte_offset: None,
        message,
        details: serde_json::json!({ "db": ctx.rel_path.to_string_lossy() }),
        provenance: None,
    }
}

/// Opens `ctx.abs_path` for parsing. The db must still hash to the manifest (`ctx.sha256`): records are
/// verified against blobs, so the chain from evidence to blob holds only for the inventoried file.
/// With a non-empty `<db>-wal` in the manifest (read through the sink, which re-hashes it), the db is
/// copied into a private temp dir together with the WAL and that copy is read; otherwise the evidence is
/// opened immutable.
pub fn open_for_parse(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<DbView, ParseError> {
    let mut wal_rel = ctx.rel_path.as_os_str().to_os_string();
    wal_rel.push("-wal");
    let wal = match sink.read_root_file(Path::new(&wal_rel), WAL_CAP) {
        Ok(Some(b)) if !b.is_empty() => Some(b),
        Ok(_) => None,
        Err(RootFileError::TooLarge) => {
            sink.anomaly(wal_anomaly(
                ctx,
                Severity::Warning,
                "WAL not applied: larger than 256 MiB".to_string(),
            ));
            None
        }
        Err(e) => {
            sink.anomaly(wal_anomaly(
                ctx,
                Severity::Warning,
                format!("WAL not applied: cannot read it ({e:?})"),
            ));
            None
        }
    };
    let Some(wal) = wal else {
        let (sha, _) = sha256_file(&ctx.abs_path)?;
        if sha != ctx.sha256 {
            return Err(ParseError::Invalid(CHANGED.to_string()));
        }
        return Ok(DbView {
            conn: open_immutable(&ctx.abs_path)?,
            wal_applied: false,
            _tmp: None,
        });
    };
    let tmp = tempfile::Builder::new().prefix("vem-cursor-").tempdir()?;
    let db: PathBuf = tmp.path().join("db");
    // Hash exactly the bytes copied, so there is no window between the check and the read.
    let (sha, _) = copy_and_hash(&ctx.abs_path, &db)?;
    if sha != ctx.sha256 {
        return Err(ParseError::Invalid(CHANGED.to_string()));
    }
    std::fs::write(tmp.path().join("db-wal"), &wal)?;
    // Our own copy, opened read-write so SQLite can create the `-shm` it needs to apply the WAL.
    let conn = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(invalid)?;
    let conn = harden(conn)?;
    conn.execute_batch("PRAGMA query_only = ON;")
        .map_err(invalid)?;
    sink.anomaly(wal_anomaly(
        ctx,
        Severity::Info,
        format!(
            "read from a WAL-applied view of this db ({} WAL bytes applied to a private copy); rows are not tagged by whether they live in the WAL",
            wal.len()
        ),
    ));
    Ok(DbView {
        conn,
        wal_applied: true,
        _tmp: Some(tmp),
    })
}

/// Whether `name` is an ordinary table: a view or a virtual table of that name counts as absent.
pub fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1 AND rootpage > 0 AND sql NOT LIKE 'CREATE VIRTUAL%'",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}
