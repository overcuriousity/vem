mod common;

use common::*;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::query::*;
use vem_case::{Case, CaseError};

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
    ingest(&mut case, None).unwrap();
    (tmp, case)
}

fn by_hid(case: &Case, hid: &str) -> SessionRow {
    sessions(case, &SessionFilter::default())
        .unwrap()
        .into_iter()
        .find(|s| s.harness_session_id == hid)
        .unwrap()
}

#[test]
fn totals_and_root_overview() {
    let (_t, case) = ingested();
    let t = totals(&case).unwrap();
    assert_eq!((t.sessions, t.messages, t.tool_calls), (5, 23, 4));
    assert_eq!(
        t.observations,
        observations(&case, &ObservationFilter::default())
            .unwrap()
            .len() as i64
    );
    assert_eq!(
        t.anomalies_info + t.anomalies_warning + t.anomalies_error,
        4
    );
    let ov = root_overviews(&case).unwrap();
    assert_eq!(ov.len(), 1);
    assert!(!ov[0].identification.is_empty());
    assert_eq!(ov[0].ingest.counts.get("parsed"), Some(&6));
    assert_eq!(ov[0].ingest.counts.get("inventoried"), Some(&4));
    assert!(ov[0].ingest.failed.is_empty());
    assert_eq!(ov[0].ingest.last_ingest.as_ref().unwrap().action, "ingest");
    let audit = audit_log(&case).unwrap();
    assert_eq!(audit.first().unwrap().action, "case.create");
}

#[test]
fn session_filters() {
    let (_t, case) = ingested();
    let all = sessions(&case, &SessionFilter::default()).unwrap();
    let with_children = sessions(
        &case,
        &SessionFilter {
            has_children: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        with_children
            .iter()
            .map(|s| s.harness_session_id.as_str())
            .collect::<Vec<_>>(),
        vec![S1]
    );
    let with_anomalies = sessions(
        &case,
        &SessionFilter {
            has_anomalies: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        with_anomalies.len(),
        all.iter().filter(|s| s.anomaly_count > 0).count()
    );
    assert!(sessions(
        &case,
        &SessionFilter {
            to: Some("2000-01-01".into()),
            ..Default::default()
        }
    )
    .unwrap()
    .is_empty());
    let dated = all
        .iter()
        .filter(|s| s.first_ts.is_some() || s.last_ts.is_some())
        .count();
    assert_eq!(
        sessions(
            &case,
            &SessionFilter {
                from: Some("2000-01-01".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        dated
    );
    // A date-only `to` includes the whole day.
    let s1 = by_hid(&case, S1);
    let day = s1.first_ts.as_ref().unwrap()[..10].to_string();
    assert!(sessions(
        &case,
        &SessionFilter {
            to: Some(day),
            ..Default::default()
        }
    )
    .unwrap()
    .iter()
    .any(|s| s.id == s1.id));
}

#[test]
fn session_detail_has_ancestors_and_children() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let child = children(&case, s1.id).unwrap().remove(0);
    let d = session_detail(&case, child.id).unwrap().unwrap();
    assert_eq!(
        d.ancestors.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![s1.id]
    );
    assert!(d.children.is_empty());
    let d1 = session_detail(&case, s1.id).unwrap().unwrap();
    assert!(d1.ancestors.is_empty());
    assert_eq!(d1.children.len(), 1);
    assert!(session_detail(&case, 999_999).unwrap().is_none());
}

#[test]
fn messages_carry_origin_and_link_to_tool_calls_and_observations() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let msgs = messages(&case, s1.id, true).unwrap();
    assert!(msgs.iter().all(|m| m.origin == "stored"));
    let deleted = by_hid(&case, "33333333-0000-4000-8000-000000000003");
    assert!(messages(&case, deleted.id, true)
        .unwrap()
        .iter()
        .all(|m| m.origin == "derived"));

    let bash_msg = msgs
        .iter()
        .find(|m| {
            m.blocks
                .iter()
                .any(|b| b.kind == "tool_use" && b.payload["name"] == "Bash")
        })
        .unwrap();
    assert_eq!(
        message(&case, bash_msg.id).unwrap().unwrap().id,
        bash_msg.id
    );
    let block = bash_msg
        .blocks
        .iter()
        .find(|b| b.kind == "tool_use")
        .unwrap();
    assert_eq!(
        message_id_for_block(&case, block.id).unwrap(),
        Some(bash_msg.id)
    );
    let tcs = tool_calls_for_message(&case, bash_msg.id).unwrap();
    assert_eq!(
        tcs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Bash"]
    );
    let result_msg = msgs
        .iter()
        .find(|m| {
            m.blocks
                .iter()
                .any(|b| b.tool_call_id == Some(tcs[0].id) && b.kind == "tool_result")
        })
        .unwrap();
    assert_eq!(
        tool_calls_for_message(&case, result_msg.id).unwrap()[0].id,
        tcs[0].id
    );
    let obs = observations_for_message(&case, bash_msg.id).unwrap();
    assert!(obs.iter().any(|o| o.kind == "command_executed"));
    assert!(message(&case, 999_999).unwrap().is_none());
}

#[test]
fn activity_rows_link_back_to_messages() {
    let (_t, case) = ingested();
    let cmds = activity(
        &case,
        &ObservationFilter {
            kinds: vec!["command_executed".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].harness_session_id, S1);
    let mid = cmds[0].message_id.unwrap();
    assert!(message(&case, mid)
        .unwrap()
        .unwrap()
        .blocks
        .iter()
        .any(|b| b.kind == "tool_use"));
    let json = serde_json::to_value(&cmds[0]).unwrap();
    assert_eq!(
        json["kind"], "command_executed",
        "observation fields are flattened"
    );
    let files = activity(
        &case,
        &ObservationFilter {
            kinds: vec!["file_written".into(), "file_edited".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(files.len(), 3);
    // A paste observation derives from a history line that is not a message of a transcript session.
    let pastes = activity(
        &case,
        &ObservationFilter {
            kinds: vec!["paste_detected".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(pastes.len(), 1);
    assert_eq!(pastes[0].message_id, None);
}

#[test]
fn search_is_enriched_and_tolerates_syntax() {
    let (_t, case) = ingested();
    let hits = search(&case, "notes file", 50).unwrap();
    assert!(!hits.is_empty());
    let m = message(&case, hits[0].message_id).unwrap().unwrap();
    assert_eq!(hits[0].message_ordinal, m.ordinal);
    assert!(search(&case, "   ", 50).unwrap().is_empty());
    for q in ["\"", "*", "(", "NEAR", "-", "a OR b", "\"*( NEAR -"] {
        search(&case, q, 50).unwrap_or_else(|e| panic!("query {q:?} failed: {e}"));
    }
}

#[test]
fn raw_window_is_verified_and_clamped() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let m = &messages(&case, s1.id, false).unwrap()[0];
    let full = raw_record(&case, m.provenance_id).unwrap();
    let w = raw_window(&case, m.provenance_id, 0, 10).unwrap();
    assert_eq!(
        (w.total_length, w.offset, w.bytes.as_slice()),
        (full.len() as u64, 0, &full[..10])
    );
    let past = raw_window(&case, m.provenance_id, full.len() as u64 + 100, 10).unwrap();
    assert!(past.bytes.is_empty());
    assert_eq!(past.offset, full.len() as u64);
    let big = raw_window(&case, m.provenance_id, 0, 10_000_000).unwrap();
    assert!(big.bytes.len() as u64 <= RAW_WINDOW_MAX);
    assert!(matches!(
        raw_window(&case, 999_999, 0, 10),
        Err(CaseError::NotFound(_))
    ));
}

#[test]
fn blob_bytes_reads_retained_content() {
    let (_t, case) = ingested();
    let written = observations(
        &case,
        &ObservationFilter {
            kind: Some("file_written".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let sha = written[0].after_blob.clone().unwrap();
    let b = blob_bytes(&case, &sha, 1024).unwrap().unwrap();
    assert_eq!(
        (b.bytes.as_slice(), b.truncated, b.size),
        (b"# Notes\n".as_slice(), false, 8)
    );
    let t = blob_bytes(&case, &sha, 3).unwrap().unwrap();
    assert_eq!((t.bytes.len(), t.truncated), (3, true));
    assert!(matches!(
        blob_bytes(&case, "../case.db", 10),
        Err(CaseError::Invalid(_))
    ));
    assert!(blob_bytes(&case, &"0".repeat(64), 10).unwrap().is_none());
}
