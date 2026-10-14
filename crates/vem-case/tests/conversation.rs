//! The conversation fixture: one API response split over several records, interleaved tool results,
//! redacted thinking, harness-injected user records and harness bookkeeping record types.

use std::path::Path;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::query::*;
use vem_case::Case;

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/conversation")
        .canonicalize()
        .unwrap();
    attach(
        &mut case,
        &root,
        AttachOptions {
            label: "bob".into(),
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

fn session_messages(case: &Case) -> Vec<MessageRow> {
    let s = &sessions(case, &SessionFilter::default()).unwrap()[0];
    messages(case, s.id, true).unwrap()
}

#[test]
fn records_of_one_api_message_share_a_turn_key() {
    let (_t, case) = ingested();
    let msgs = session_messages(&case);
    let keys: Vec<&str> = msgs
        .iter()
        .filter(|m| m.role == "assistant")
        .map(|m| m.turn_key.as_deref().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "msg_A|req_A",
            "msg_A|req_A",
            "msg_A|req_A",
            "msg_A|req_A",
            "msg_B|req_B"
        ]
    );
    assert!(msgs
        .iter()
        .filter(|m| m.role != "assistant")
        .all(|m| m.turn_key.is_none()));
    // The API message metadata is kept, without its content (that lives in the blocks).
    let a = msgs.iter().find(|m| m.role == "assistant").unwrap();
    assert_eq!(a.attributes["message"]["id"], "msg_A");
    assert_eq!(
        a.attributes["message"]["stop_reason"],
        serde_json::Value::Null
    );
    assert!(a.attributes["message"].get("content").is_none());
}

#[test]
fn turn_key_falls_back_to_request_id_for_older_cases() {
    let attrs = serde_json::json!({ "requestId": "req_9" });
    assert_eq!(turn_key("assistant", &attrs).as_deref(), Some("|req_9"));
    assert_eq!(turn_key("assistant", &serde_json::json!({})), None);
    assert_eq!(turn_key("user", &attrs), None);
}

#[test]
fn harness_injected_user_records_are_classified() {
    let (_t, case) = ingested();
    let msgs = session_messages(&case);
    let classes: Vec<&str> = msgs
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.content_class.as_str())
        .collect();
    assert_eq!(
        classes,
        [
            "caveat",
            "command",
            "command_output",
            "paste",
            "injected",
            "injected"
        ]
    );
}

#[test]
fn every_tool_result_is_kept_and_paired_or_orphaned() {
    let (_t, case) = ingested();
    let msgs = session_messages(&case);
    let s = &sessions(&case, &SessionFilter::default()).unwrap()[0];
    let calls = tool_calls(&case, s.id).unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c.tool_result_block_id.is_some()));
    assert!(calls.iter().any(|c| c.is_error));
    let results: Vec<&BlockRow> = msgs
        .iter()
        .filter(|m| m.content_class == "tool_result")
        .flat_map(|m| &m.blocks)
        .collect();
    assert_eq!(results.len(), 3);
    let orphan = results
        .iter()
        .find(|b| b.tool_use_id.as_deref() == Some("toolu_missing"))
        .unwrap();
    assert_eq!(orphan.tool_call_id, None);
}

#[test]
fn bookkeeping_record_types_are_known_and_counts_are_labelled() {
    let (_t, case) = ingested();
    let unknown = anomalies(
        &case,
        &AnomalyFilter {
            kind: Some("unknown_record_type".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(unknown.len(), 2);
    assert!(unknown.iter().all(|a| a.message.contains("widget-state")));
    let msgs = session_messages(&case);
    for t in ["cost-state", "pr-link"] {
        assert!(msgs
            .iter()
            .any(|m| m.harness_record_type == t && m.role == "meta"));
    }
    let ov = &root_overviews(&case).unwrap()[0];
    assert_eq!(ov.ingest.files, ov.ingest.counts.values().sum::<i64>());
    assert_eq!(
        ov.ingest.files_in_stores,
        ov.stores.iter().map(|s| s.file_count).sum::<i64>()
    );
    assert_eq!(ov.ingest.records, msgs.len() as i64);
}

#[test]
fn vem_turn_groups_assistant_records() {
    let attrs = serde_json::json!({ "vem_turn": "t1" });
    assert_eq!(turn_key("assistant", &attrs).as_deref(), Some("t1"));
    assert_eq!(turn_key("user", &attrs), None);
}

#[test]
fn vem_content_class_overrides() {
    let block = BlockRow {
        id: 1,
        ordinal: 0,
        kind: "text".into(),
        text: Some("<user_query>x".into()),
        payload: serde_json::Value::Null,
        tool_use_id: None,
        tool_call_id: None,
    };
    let blocks = [block];
    let injected = serde_json::json!({ "vem_content_class": "injected" });
    assert_eq!(content_class("user", &injected, &blocks), "injected");
    let bogus = serde_json::json!({ "vem_content_class": "bogus" });
    assert_eq!(content_class("user", &bogus, &blocks), "prompt");
}
