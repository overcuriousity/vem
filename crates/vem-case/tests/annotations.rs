mod common;

use common::*;
use vem_case::annotations::{self, Annotation};
use vem_case::evidence::{attach, AttachOptions};
use vem_case::{query, Case, CaseError};

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(
        &mut case,
        &fixture_root(),
        AttachOptions {
            label: "alice".into(),
            host: None,
            user: None,
            os: None,
            harness: None,
            retain: true,
        },
    )
    .unwrap();
    vem_case::ingest::ingest(&mut case, None).unwrap();
    (tmp, case)
}

fn audits(case: &Case, action: &str) -> Vec<serde_json::Value> {
    let mut stmt = case
        .conn
        .prepare("SELECT details FROM audit_log WHERE action = ?1 ORDER BY id")
        .unwrap();
    stmt.query_map([action], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|d| serde_json::from_str(&d.unwrap()).unwrap())
        .collect()
}

fn first_message(case: &Case) -> query::MessageRow {
    let s = query::sessions(case, &query::SessionFilter::default())
        .unwrap()
        .into_iter()
        .find(|s| s.harness_session_id == S1)
        .unwrap();
    query::messages(case, s.id, false).unwrap().remove(0)
}

#[test]
fn create_list_delete_are_audited() {
    let (_t, case) = ingested();
    let m = first_message(&case);
    let a = annotations::create(
        &case,
        "message",
        m.id,
        "note",
        "first prompt of the incident",
    )
    .unwrap();
    assert_eq!(
        (a.target_type.as_str(), a.target_id, a.kind.as_str()),
        ("message", m.id, "note")
    );
    assert!(a.created_at.ends_with('Z'));
    let b = annotations::create(&case, "block", m.blocks[0].id, "tag", "exfil").unwrap();
    assert_eq!(
        annotations::list(&case, Some("message"), Some(m.id)).unwrap(),
        vec![a.clone()]
    );
    assert_eq!(annotations::list(&case, None, None).unwrap().len(), 2);
    let for_msg: Vec<Annotation> = annotations::list_for_message(&case, m.id).unwrap();
    assert_eq!(
        for_msg.iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![a.id, b.id]
    );
    annotations::delete(&case, a.id).unwrap();
    assert_eq!(annotations::list(&case, None, None).unwrap(), vec![b]);
    assert!(matches!(
        annotations::delete(&case, a.id),
        Err(CaseError::NotFound(_))
    ));
    let created = audits(&case, "annotation.create");
    assert_eq!(created.len(), 2);
    assert_eq!(created[0]["value"], "first prompt of the incident");
    let deleted = audits(&case, "annotation.delete");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0]["id"], a.id);
    assert_eq!(
        deleted[0]["value"], "first prompt of the incident",
        "the audit log alone can rebuild the annotation"
    );
}

#[test]
fn invalid_annotations_are_refused_without_audit() {
    let (_t, case) = ingested();
    let m = first_message(&case);
    let before = audits(&case, "annotation.create").len();
    for (tt, id, kind, value) in [
        ("message", 999_999, "note", "x"),
        ("planet", m.id, "note", "x"),
        ("message", m.id, "shout", "x"),
        ("message", m.id, "note", "   "),
    ] {
        assert!(
            matches!(
                annotations::create(&case, tt, id, kind, value),
                Err(CaseError::Invalid(_))
            ),
            "{tt} {id} {kind} {value:?}"
        );
    }
    let huge = "x".repeat(annotations::MAX_VALUE_LEN + 1);
    assert!(matches!(
        annotations::create(&case, "message", m.id, "note", &huge),
        Err(CaseError::Invalid(_))
    ));
    assert_eq!(audits(&case, "annotation.create").len(), before);
    assert!(annotations::list(&case, None, None).unwrap().is_empty());
}
