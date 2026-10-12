//! The experimental `chats/<ws>/<agent>/store.db` parser: hex meta, blob tree order, JSON messages,
//! derived protobuf text, and the integrity rules for unreferenced or cyclic blobs.

mod cursor_common;
use cursor_common::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use vem_adapters::cursor::CursorAdapter;
use vem_core::adapter::ParseOutcome;
use vem_core::model::*;
use vem_core::sink::ParseSink;
use vem_core::testing::VecSink;

const S5: &str = "1c1c1c1c-0000-4000-8000-000000000005";
const FIXTURE_DB: &str = "chats/ws1/1c1c1c1c-0000-4000-8000-000000000005/store.db";

fn id(n: u8) -> String {
    hex(&[n; 32])
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// A protobuf node whose field 1 repeats the 32-byte refs `to`, plus a varint field 2.
fn node(to: &[String]) -> Vec<u8> {
    let mut b = Vec::new();
    for r in to {
        b.extend_from_slice(&[0x0a, 0x20]);
        b.extend_from_slice(&unhex(r));
    }
    b.extend_from_slice(&[0x10, 0x01]);
    b
}

fn meta_hex(agent: &str, root: &str) -> String {
    hex(json!({ "agentId": agent, "latestRootBlobId": root, "name": "t", "createdAt": 1767225600000u64 })
        .to_string()
        .as_bytes())
}

/// A `.cursor`-shaped temp root with `chats/ws/<dir>/store.db`. `meta: None` leaves the table empty;
/// `with_blobs: false` creates no `blobs` table.
fn temp_store(
    dir: &str,
    meta: Option<&str>,
    blobs: &[(String, Vec<u8>)],
    with_blobs: bool,
) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().unwrap();
    let rel = format!("chats/ws/{dir}/store.db");
    let path = tmp.path().join(&rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);")
        .unwrap();
    if with_blobs {
        conn.execute_batch("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);")
            .unwrap();
    }
    if let Some(m) = meta {
        conn.execute("INSERT INTO meta VALUES ('0', ?1)", [m])
            .unwrap();
    }
    for (k, v) in blobs {
        conn.execute("INSERT INTO blobs VALUES (?1, ?2)", params![k, v])
            .unwrap();
    }
    drop(conn);
    (tmp, rel)
}

fn fixture_parse() -> (VecSink, ParseOutcome) {
    let mut sink = VecSink::default();
    let out = parse_cursor(&fixture("dotcursor"), FIXTURE_DB, &mut sink);
    (sink, out)
}

fn texts(m: &MessageDraft) -> Vec<String> {
    m.blocks.iter().filter_map(|b| b.text.clone()).collect()
}

#[test]
fn store_session_from_hex_meta() {
    let (sink, out) = fixture_parse();
    assert!(matches!(out, ParseOutcome::Parsed { .. }), "{out:?}");
    assert_eq!(sink.sessions.len(), 1);
    let (h, s) = &sink.sessions[0];
    assert_eq!(s.harness_session_id, S5);
    assert_eq!(s.kind, SessionKind::Primary);
    assert_eq!(s.title.as_deref(), Some("list the files"));
    let first = s.first_ts.as_ref().unwrap();
    assert_eq!(first.value.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    assert_eq!(first.origin, TsOrigin::Stored);
    assert_eq!(s.last_ts.as_ref().unwrap().origin, TsOrigin::FileMtime);
    assert!(sink
        .updates
        .iter()
        .any(|(u, x)| u == h && x.model.as_deref() == Some("claude-4-sonnet")));
    let claims: Vec<_> = sink
        .claims
        .iter()
        .map(|(_, c)| (c.scheme.as_str(), c.claimed_id.as_str()))
        .collect();
    assert_eq!(claims, vec![("cursor:composerId", S5)]);
    // The meta row is a meta message carrying the decoded fields and the tree-node count.
    let meta = &sink.messages[0].2;
    assert_eq!(meta.role, Role::Meta);
    assert_eq!(meta.harness_record_type, "meta");
    assert_eq!(meta.attributes["mode"], "agent");
    assert_eq!(meta.attributes["tree_nodes"], 1);
    assert_eq!(meta.provenance.locator.as_deref(), Some("meta:0"));

    // A directory name that differs from agentId is claimed too.
    let (tmp, rel) = temp_store("dir-x", Some(&meta_hex("agent-y", "none")), &[], true);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.sessions[0].1.harness_session_id, "agent-y");
    let claims: Vec<_> = sink
        .claims
        .iter()
        .map(|(_, c)| (c.scheme.as_str(), c.claimed_id.as_str()))
        .collect();
    assert_eq!(
        claims,
        vec![
            ("cursor:composerId", "agent-y"),
            ("cursor:store-session", "dir-x")
        ]
    );
}

#[test]
fn json_blobs_become_messages_in_root_order() {
    let (sink, _) = fixture_parse();
    let msgs: Vec<&MessageDraft> = sink.messages.iter().map(|(_, _, m)| m).skip(1).collect();
    let roles: Vec<Role> = msgs.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![Role::User, Role::Assistant, Role::Tool, Role::Meta]
    );
    assert_eq!(texts(msgs[0]), vec!["list the files"]);
    assert_eq!(
        msgs[0].provenance.locator,
        Some(format!("blobs:{}", id(0x11)))
    );
    assert_eq!(msgs[0].provenance.origin, ProvOrigin::Stored);
    assert_eq!(msgs[0].timestamp.origin, TsOrigin::Absent);
    let kinds: Vec<BlockKind> = msgs[1].blocks.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        vec![BlockKind::Thinking, BlockKind::Text, BlockKind::ToolUse]
    );
    // The user message opens a turn; the assistant and tool messages carry it.
    let turn = msgs[1].attributes["vem_turn"].clone();
    assert!(turn.as_str().unwrap().starts_with(S5));
    assert_eq!(msgs[2].attributes["vem_turn"], turn);
    assert!(msgs[0].attributes.get("vem_turn").is_none());
    // The record bytes are the blob's.
    let blob = &sink.blobs[&msgs[0].provenance.content_sha256];
    assert_eq!(
        serde_json::from_slice::<Value>(blob).unwrap()["role"],
        "user"
    );
    assert!(sink
        .anomalies_of(AnomalyKind::UnreferencedRecord)
        .is_empty());
}

#[test]
fn tool_call_and_result_pair_by_tool_call_id() {
    let (sink, _) = fixture_parse();
    assert_eq!(sink.tool_calls.len(), 1);
    let tc = &sink.tool_calls[0].2;
    assert_eq!(tc.name, "run_terminal_cmd");
    assert_eq!(tc.category, ToolCategory::Shell);
    assert_eq!(tc.input, json!({ "command": "ls" }));
    assert_eq!(tc.result_text.as_deref(), Some("a.txt\nb.txt"));
    let result = tc.tool_result.expect("paired");
    let (_, _, m) = sink
        .messages
        .iter()
        .find(|(_, h, _)| *h == result.message)
        .unwrap();
    assert_eq!(m.role, Role::Tool);
    assert_eq!(
        m.blocks[result.ordinal as usize].kind,
        BlockKind::ToolResult
    );
    assert_eq!(
        m.blocks[result.ordinal as usize].tool_use_id.as_deref(),
        Some("call_1")
    );
    assert!(sink
        .observations
        .iter()
        .any(|(_, o)| o.kind == ObservationKind::CommandExecuted));
    assert!(sink
        .anomalies_of(AnomalyKind::UnpairedToolResult)
        .is_empty());
}

#[test]
fn protobuf_blob_is_derived_low_meta() {
    let (sink, _) = fixture_parse();
    let m = &sink.messages.last().unwrap().2;
    assert_eq!(m.role, Role::Meta);
    assert_eq!(m.provenance.origin, ProvOrigin::Derived);
    assert_eq!(m.harness_record_type, "protobuf");
    assert_eq!(m.attributes["vem_confidence"], "low");
    assert_eq!(m.attributes["harness_record_type"], "protobuf");
    assert_eq!(texts(m), vec!["workspace summary"]);

    // Bytes with no strings in them become one Other block with the length.
    let r = id(0xee);
    let (tmp, rel) = temp_store(
        "d",
        Some(&meta_hex("d", &r)),
        &[(r.clone(), vec![0x08, 0x01, 0x10, 0x02, 0x1d, 1, 2, 3, 4])],
        true,
    );
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let m = &sink.messages.last().unwrap().2;
    assert_eq!(m.blocks.len(), 1);
    assert_eq!(m.blocks[0].kind, BlockKind::Other);
    assert_eq!(m.blocks[0].payload, json!({ "bytes": 9 }));
}

#[test]
fn unreferenced_blob_appended_with_anomaly() {
    let (root, a, orphan) = (id(1), id(2), id(3));
    let user = |t: &str| {
        json!({ "role": "user", "content": t })
            .to_string()
            .into_bytes()
    };
    let (tmp, rel) = temp_store(
        "d",
        Some(&meta_hex("d", &root)),
        &[
            (orphan.clone(), user("orphan")),
            (root.clone(), node(std::slice::from_ref(&a))),
            (a.clone(), user("listed")),
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let users: Vec<Vec<String>> = sink
        .messages_with_role(Role::User)
        .into_iter()
        .map(texts)
        .collect();
    assert_eq!(
        users,
        vec![vec!["listed".to_string()], vec!["orphan".to_string()]]
    );
    let un = sink.anomalies_of(AnomalyKind::UnreferencedRecord);
    assert_eq!(un.len(), 1);
    assert_eq!(un[0].severity, Severity::Info);
    assert_eq!(
        un[0].provenance.as_ref().unwrap().locator,
        Some(format!("blobs:{orphan}"))
    );
}

#[test]
fn cyclic_refs_terminate() {
    let (a, b) = (id(0xa1), id(0xb2));
    let (tmp, rel) = temp_store(
        "d",
        Some(&meta_hex("d", &a)),
        &[
            (a.clone(), node(std::slice::from_ref(&b))),
            (b.clone(), node(&[a.clone(), b.clone()])),
        ],
        true,
    );
    let mut sink = VecSink::default();
    let out = parse_cursor(tmp.path(), &rel, &mut sink);
    assert!(matches!(out, ParseOutcome::Parsed { .. }));
    assert_eq!(sink.messages.len(), 1, "only the meta message");
    assert_eq!(sink.messages[0].2.attributes["tree_nodes"], 2);
    assert!(sink
        .anomalies_of(AnomalyKind::UnreferencedRecord)
        .is_empty());
}

#[test]
fn bad_hex_meta_is_malformed_not_fatal() {
    let a = id(7);
    let (tmp, rel) = temp_store(
        "dir-z",
        Some("zz-not-hex"),
        &[(
            a,
            json!({ "role": "user", "content": "hi" })
                .to_string()
                .into_bytes(),
        )],
        true,
    );
    let mut sink = VecSink::default();
    let out = parse_cursor(tmp.path(), &rel, &mut sink);
    assert!(matches!(out, ParseOutcome::Parsed { .. }));
    assert_eq!(sink.anomalies_of(AnomalyKind::MalformedRecord).len(), 1);
    assert_eq!(sink.sessions[0].1.harness_session_id, "dir-z");
    assert!(sink.find_session("dir-z").is_some());
    assert_eq!(texts(sink.messages_with_role(Role::User)[0]), vec!["hi"]);
    // Without a root, order is unknown: nothing is called unreferenced.
    assert!(sink
        .anomalies_of(AnomalyKind::UnreferencedRecord)
        .is_empty());
    // Hex of something that is not JSON is malformed too.
    let (tmp, rel) = temp_store("d", Some(&hex(b"not json")), &[], true);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.anomalies_of(AnomalyKind::MalformedRecord).len(), 1);
}

#[test]
fn missing_blobs_table_is_unknown_generation() {
    let (tmp, rel) = temp_store("d", Some(&meta_hex("d", "x")), &[], false);
    let mut sink = VecSink::default();
    let out = with_ctx(&CursorAdapter, tmp.path(), &rel, 1, |ctx| {
        vem_adapters::cursor::chat_store::parse_store_db(ctx, &mut sink).unwrap()
    });
    assert!(matches!(out, ParseOutcome::NotParsed));
    assert_eq!(
        sink.anomalies_of(AnomalyKind::UnknownStoreGeneration).len(),
        1
    );
    assert!(sink.sessions.is_empty());
}
