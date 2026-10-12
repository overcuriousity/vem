//! The safe SQLite open used on Cursor evidence: immutable, or a WAL-applied private copy.

use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vem_adapters::cursor::sqlite::*;
use vem_core::adapter::{FileContext, ParseError, StoreCandidate};
use vem_core::hash::sha256_file;
use vem_core::model::{AnomalyKind, Harness, Severity, SourceFileHandle};
use vem_core::testing::VecSink;

/// Name -> bytes of every entry in `dir` (flat).
fn listing(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().to_string(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

/// A WAL-mode db `state.vscdb` in its own dir with row `a` checkpointed into the main file and, when
/// `wal_row`, row `b` only in `state.vscdb-wal` (copied out while the writer was still open).
fn wal_db(wal_row: bool) -> (tempfile::TempDir, PathBuf) {
    let build = tempfile::tempdir().unwrap();
    let src = build.path().join("state.vscdb");
    let conn = Connection::open(&src).unwrap();
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
         INSERT INTO cursorDiskKV VALUES ('a', 'main');
         PRAGMA wal_checkpoint(TRUNCATE);
         PRAGMA wal_autocheckpoint = 0;",
    )
    .unwrap();
    if wal_row {
        conn.execute("INSERT INTO cursorDiskKV VALUES ('b', 'wal')", [])
            .unwrap();
    }
    let ev = tempfile::tempdir().unwrap();
    let db = ev.path().join("state.vscdb");
    std::fs::copy(&src, &db).unwrap();
    if wal_row {
        std::fs::copy(
            build.path().join("state.vscdb-wal"),
            ev.path().join("state.vscdb-wal"),
        )
        .unwrap();
    }
    drop(conn);
    (ev, db)
}

fn store() -> StoreCandidate {
    StoreCandidate {
        kind: "cursor:state-vscdb".into(),
        generation: Some("cursor-disk-kv".into()),
        rel_path: PathBuf::from("User/globalStorage"),
        files: vec![],
    }
}

fn keys(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT key FROM cursorDiskKV ORDER BY key")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Calls `open_for_parse` on `db` as root file `state.vscdb`, with the manifest hash `sha` (the
/// file's real hash when `None`).
fn open(db: &Path, sink: &mut VecSink, sha: Option<&str>) -> Result<DbView, ParseError> {
    let store = store();
    let real = sha256_file(db).map(|(s, _)| s).unwrap_or_default();
    let rel = PathBuf::from("state.vscdb");
    let ctx = FileContext {
        root: db.parent().unwrap(),
        store: &store,
        rel_path: &rel,
        abs_path: db.to_path_buf(),
        handle: SourceFileHandle(1),
        mtime: None,
        sha256: sha.unwrap_or(&real),
    };
    open_for_parse(&ctx, sink)
}

#[test]
fn immutable_open_creates_no_sidecars() {
    let (ev, db) = wal_db(false);
    let before = listing(ev.path());
    assert_eq!(before.len(), 1, "only the checkpointed db");
    let conn = open_immutable(&db).unwrap();
    assert_eq!(keys(&conn), vec!["a"]);
    drop(conn);
    assert_eq!(listing(ev.path()), before);
}

#[test]
fn wal_rows_are_visible() {
    let (ev, db) = wal_db(true);
    let before = listing(ev.path());
    let mut sink = VecSink::default();
    sink.root_files.insert(
        PathBuf::from("state.vscdb-wal"),
        std::fs::read(ev.path().join("state.vscdb-wal")).unwrap(),
    );
    let view = open(&db, &mut sink, None).unwrap();
    assert!(view.wal_applied);
    assert_eq!(keys(&view.conn), vec!["a", "b"]);
    let wal = sink.anomalies_of(AnomalyKind::WalApplied);
    assert_eq!(wal.len(), 1);
    assert_eq!(wal[0].severity, Severity::Info);
    assert!(
        wal[0].message.contains("WAL-applied view"),
        "{}",
        wal[0].message
    );
    drop(view);
    assert_eq!(listing(ev.path()), before, "evidence untouched");
}

#[test]
fn oversized_wal_falls_back() {
    let (_ev, db) = wal_db(true);
    let mut sink = VecSink::default();
    // Zeroed and never written, so the pages are not committed.
    sink.root_files.insert(
        PathBuf::from("state.vscdb-wal"),
        vec![0u8; WAL_CAP as usize + 1],
    );
    let view = open(&db, &mut sink, None).unwrap();
    assert!(!view.wal_applied);
    assert_eq!(keys(&view.conn), vec!["a"]);
    let wal = sink.anomalies_of(AnomalyKind::WalApplied);
    assert_eq!(wal.len(), 1);
    assert_eq!(wal[0].severity, Severity::Warning);
    assert_eq!(wal[0].message, "WAL not applied: larger than 256 MiB");
}

#[test]
fn view_named_like_table_is_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("state.vscdb");
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE ItemTable (key TEXT, value BLOB);
             CREATE VIEW cursorDiskKV AS SELECT 1;",
        )
        .unwrap();
    let c = open_immutable(&db).unwrap();
    assert!(has_table(&c, "ItemTable"));
    assert!(!has_table(&c, "cursorDiskKV"));
    assert!(!has_table(&c, "nope"));
}

#[test]
fn garbage_db_fails_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("store.db");
    // Deterministic pseudo-random bytes.
    let mut x: u32 = 0x9e37_79b9;
    let bytes: Vec<u8> = (0..4096)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    std::fs::write(&db, bytes).unwrap();
    let mut sink = VecSink::default();
    match open(&db, &mut sink, None) {
        Err(ParseError::Invalid(_)) => {}
        Err(e) => panic!("expected Invalid, got {e:?}"),
        Ok(v) => assert!(
            v.conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                    .get::<_, i64>(0))
                .is_err(),
            "a garbage file must not read as a database"
        ),
    }
    if let Ok(c) = open_immutable(&db) {
        assert!(!has_table(&c, "cursorDiskKV"));
    }
}

#[test]
fn db_changed_since_inventory_fails() {
    let (ev, db) = wal_db(true);
    let other = "0".repeat(64);
    // Both paths: immutable (no WAL in the manifest) and WAL-applied.
    let mut sink = VecSink::default();
    match open(&db, &mut sink, Some(&other)) {
        Err(ParseError::Invalid(m)) => assert_eq!(m, "evidence changed since inventory"),
        r => panic!("expected Invalid, got {:?}", r.map(|v| v.wal_applied)),
    }
    let mut sink = VecSink::default();
    sink.root_files.insert(
        PathBuf::from("state.vscdb-wal"),
        std::fs::read(ev.path().join("state.vscdb-wal")).unwrap(),
    );
    match open(&db, &mut sink, Some(&other)) {
        Err(ParseError::Invalid(m)) => assert_eq!(m, "evidence changed since inventory"),
        r => panic!("expected Invalid, got {:?}", r.map(|v| v.wal_applied)),
    }
}

#[test]
fn sqlite_uri_escapes() {
    assert_eq!(
        sqlite_uri(Path::new("/a b/x?y#z")),
        "file:/a%20b/x%3Fy%23z?mode=ro&immutable=1"
    );
    assert_eq!(
        sqlite_uri(Path::new("/A-z_0.9~/%")),
        "file:/A-z_0.9~/%25?mode=ro&immutable=1"
    );
}

#[cfg(unix)]
#[test]
fn sqlite_uri_escapes_non_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let p = Path::new(std::ffi::OsStr::from_bytes(b"/x\xff"));
    assert_eq!(sqlite_uri(p), "file:/x%FF?mode=ro&immutable=1");
}

#[test]
fn adapters_registered() {
    let hs: Vec<Harness> = vem_adapters::adapters()
        .iter()
        .map(|a| a.harness())
        .collect();
    assert!(hs.contains(&Harness::Cursor));
    assert!(hs.contains(&Harness::CursorIde));
}
