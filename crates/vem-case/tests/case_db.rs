use vem_case::{Case, CaseError};

#[test]
fn create_then_open_round_trips_case_info() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("case1");
    let case = Case::create(&dir, "Incident 42", Some("examiner a")).unwrap();
    assert!(dir.join("case.db").is_file());
    assert!(dir.join("blobs").is_dir());
    assert!(dir.join("exports").is_dir());
    drop(case);
    let case = Case::open(&dir).unwrap();
    let info = case.info().unwrap();
    assert_eq!(info.name, "Incident 42");
    assert_eq!(info.examiner.as_deref(), Some("examiner a"));
    assert_eq!(info.tool_version, vem_case::TOOL_VERSION);
    let v: i64 = case
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, vem_case::db::SCHEMA_VERSION);
}

#[test]
fn refuses_non_empty_dir_and_non_case_dir() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("something.txt"), "x").unwrap();
    assert!(matches!(
        Case::create(tmp.path(), "n", None),
        Err(CaseError::AlreadyExists(_))
    ));
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        Case::open(empty.path()),
        Err(CaseError::NotACase(_))
    ));
}

#[test]
fn audit_log_is_append_only() {
    let tmp = tempfile::tempdir().unwrap();
    let case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    case.audit("test.action", Some("t"), serde_json::json!({"k": 1}))
        .unwrap();
    let n: i64 = case
        .conn
        .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2, "case.create plus test.action");
    assert!(case
        .conn
        .execute("UPDATE audit_log SET action = 'x'", [])
        .is_err());
    assert!(case.conn.execute("DELETE FROM audit_log", []).is_err());
    let still: i64 = case
        .conn
        .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(still, 2);
}

#[test]
fn blobs_are_content_addressed_and_deduplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let a = case.put_blob(b"hello").unwrap();
    let b = case.put_blob(b"hello").unwrap();
    assert_eq!(a, b);
    assert_eq!(a, vem_core::hash::sha256_hex(b"hello"));
    assert_eq!(std::fs::read(case.blob_path(&a)).unwrap(), b"hello");
    let n: i64 = case
        .conn
        .query_row("SELECT COUNT(*) FROM blobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}
