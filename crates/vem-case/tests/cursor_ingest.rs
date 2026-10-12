//! Cursor end to end: a `.cursor` directory and Cursor's IDE directory in one case, the case-wide
//! `cursor:` identity-claim pass, and the read-only guarantee for SQLite evidence.

mod common;

use common::copy_dir;
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::{events, Scope};
use vem_case::ingest::ingest;
use vem_case::verify::verify;
use vem_case::Case;
use vem_core::hash::sha256_file;

const T1: &str = "1c1c1c1c-0000-4000-8000-000000000001";

fn cursor_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/cursor")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn new_case() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    (tmp, case)
}

fn add(case: &mut Case, root: &Path) -> i64 {
    attach(
        case,
        root,
        AttachOptions {
            label: root.file_name().unwrap().to_string_lossy().into_owned(),
            host: None,
            user: None,
            os: None,
            harness: None,
            retain: true,
        },
    )
    .unwrap()
    .root_id
}

fn count(case: &Case, sql: &str) -> i64 {
    case.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// `(claim session, harness, join_status, matched session)` of every `cursor:composerId = T1` claim.
fn t1_claims(case: &Case) -> Vec<(i64, String, String, Option<i64>)> {
    let mut stmt = case
        .conn
        .prepare(
            "SELECT c.session_id, st.harness, c.join_status, c.matched_session_id
             FROM identity_claims c JOIN sessions s ON s.id = c.session_id JOIN stores st ON st.id = s.store_id
             WHERE c.scheme = 'cursor:composerId' AND c.claimed_id = ?1 ORDER BY st.harness, c.id",
        )
        .unwrap();
    let rows = stmt
        .query_map([T1], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

fn assert_matched_pair(case: &Case) {
    let claims = t1_claims(case);
    assert_eq!(claims.len(), 2, "{claims:?}");
    let (a, b) = (&claims[0], &claims[1]);
    assert_eq!((a.1.as_str(), b.1.as_str()), ("cursor", "cursor-ide"));
    assert_eq!(a.2, "matched", "{claims:?}");
    assert_eq!(b.2, "matched", "{claims:?}");
    assert_eq!(a.3, Some(b.0));
    assert_eq!(b.3, Some(a.0));
}

#[test]
fn cursor_roots_ingest_without_failed_files() {
    let (_tmp, mut case) = new_case();
    add(&mut case, &cursor_fixture("dotcursor"));
    add(&mut case, &cursor_fixture("ide"));
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(report.files_failed, 0);
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM source_files WHERE parse_status = 'failed'"
        ),
        0
    );
    for harness in ["cursor", "cursor-ide"] {
        let n: i64 = case
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions s JOIN stores st ON st.id = s.store_id WHERE st.harness = ?1",
                [harness],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 0, "no {harness} sessions");
    }
    // The chat store parsed into its own session.
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM sessions WHERE harness_session_id = '1c1c1c1c-0000-4000-8000-000000000005'"
        ),
        1
    );
}

#[test]
fn claims_match_across_roots_in_either_order() {
    for order in [["dotcursor", "ide"], ["ide", "dotcursor"]] {
        let (_tmp, mut case) = new_case();
        add(&mut case, &cursor_fixture(order[0]));
        ingest(&mut case, None).unwrap();
        let first = t1_claims(&case);
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0].2, "unmatched",
            "{order:?}: alone, nothing to match"
        );

        // The second root is ingested on its own: the first root's claim flips without re-parsing it.
        let second = add(&mut case, &cursor_fixture(order[1]));
        ingest(&mut case, Some(second)).unwrap();
        assert_matched_pair(&case);

        let claims = count(&case, "SELECT COUNT(*) FROM identity_claims");
        ingest(&mut case, None).unwrap();
        assert_eq!(count(&case, "SELECT COUNT(*) FROM identity_claims"), claims);
        assert_matched_pair(&case);
    }
}

#[test]
fn ambiguous_when_two_candidates() {
    let (tmp, mut case) = new_case();
    let copy = tmp.path().join("dotcursor-copy");
    copy_dir(&cursor_fixture("dotcursor"), &copy);
    add(&mut case, &cursor_fixture("dotcursor"));
    add(&mut case, &cursor_fixture("ide"));
    add(&mut case, &copy);
    ingest(&mut case, None).unwrap();
    let claims = t1_claims(&case);
    assert_eq!(claims.len(), 3);
    for c in &claims {
        assert_eq!(c.2, "ambiguous", "{claims:?}");
        assert_eq!(c.3, None);
    }
    ingest(&mut case, None).unwrap();
    assert!(t1_claims(&case).iter().all(|c| c.2 == "ambiguous"));
}

/// rel path -> (size, mtime ns, ctime ns, sha256) of every regular file under `root`.
fn snapshot(root: &Path) -> BTreeMap<String, (u64, i128, i128, String)> {
    walkdir::WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let m = e.metadata().unwrap();
            let ns = |s: i64, n: i64| i128::from(s) * 1_000_000_000 + i128::from(n);
            (
                e.path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                (
                    m.len(),
                    ns(m.mtime(), m.mtime_nsec()),
                    ns(m.ctime(), m.ctime_nsec()),
                    sha256_file(e.path()).unwrap().0,
                ),
            )
        })
        .collect()
}

#[test]
fn evidence_tree_unchanged() {
    let roots = [cursor_fixture("dotcursor"), cursor_fixture("ide")];
    let before: Vec<_> = roots.iter().map(|r| snapshot(r)).collect();
    let (_tmp, mut case) = new_case();
    for r in &roots {
        add(&mut case, r);
    }
    ingest(&mut case, None).unwrap();
    verify(&mut case).unwrap();
    let after: Vec<_> = roots.iter().map(|r| snapshot(r)).collect();
    // Same files (so no `-shm` or `-wal` was created), same bytes, size, mtime and ctime: the manifest
    // hash read and the immutable opens are plain reads.
    assert_eq!(before, after);
    let paths = |suffix: &str| -> Vec<String> {
        after
            .iter()
            .flat_map(|s| s.keys().filter(|p| p.ends_with(suffix)).cloned())
            .collect()
    };
    assert!(paths("-shm").is_empty());
    assert_eq!(paths("-wal"), vec!["User/globalStorage/state.vscdb-wal"]);
}

#[test]
fn verify_reports_no_drift_on_cursor_fixtures() {
    let (_tmp, mut case) = new_case();
    add(&mut case, &cursor_fixture("dotcursor"));
    add(&mut case, &cursor_fixture("ide"));
    ingest(&mut case, None).unwrap();
    let r = verify(&mut case).unwrap();
    assert!(r.files_checked > 0);
    assert!(r.drifted.is_empty(), "{:?}", r.drifted);
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert!(r.blobs_checked > 0);
    assert!(r.blob_errors.is_empty(), "{:?}", r.blob_errors);
    assert!(r.unindexed_blobs.is_empty(), "{:?}", r.unindexed_blobs);
}

#[test]
fn export_timesketch_source_is_ai_cursor() {
    let (_tmp, mut case) = new_case();
    let a = add(&mut case, &cursor_fixture("dotcursor"));
    let b = add(&mut case, &cursor_fixture("ide"));
    ingest(&mut case, None).unwrap();
    for (root, source) in [(a, "AI:CURSOR"), (b, "AI:CURSOR_IDE")] {
        let ev = events(&case, &Scope::Root(root)).unwrap();
        assert!(!ev.is_empty());
        assert!(ev.iter().all(|e| e.source == source), "root {root}");
    }
}
