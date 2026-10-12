//! Cursor's global `state.vscdb` (`cursorDiskKV`): composers, bubbles, tool calls and the integrity
//! rules for deleted conversation parts.

mod cursor_common;
use cursor_common::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use vem_adapters::cursor::kv::parse_state_vscdb_with_cap;
use vem_adapters::cursor::CursorIdeAdapter;
use vem_core::adapter::ParseOutcome;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::testing::VecSink;

const DB: &str = "User/globalStorage/state.vscdb";
const C1: &str = "1c1c1c1c-0000-4000-8000-000000000001";
const C2: &str = "1c1c1c1c-0000-4000-8000-000000000002";
const C3: &str = "1c1c1c1c-0000-4000-8000-000000000003";
const C4: &str = "1c1c1c1c-0000-4000-8000-000000000004";
const C9: &str = "1c1c1c1c-0000-4000-8000-000000000009";

fn b(n: u32) -> String {
    format!("b1b1b1b1-0000-4000-8000-{n:012}")
}

fn global() -> (VecSink, ParseOutcome) {
    let mut sink = VecSink::default();
    let out = parse_ide(&fixture("ide"), DB, 1, &mut sink);
    (sink, out)
}

fn session(sink: &VecSink, id: &str) -> SessionHandle {
    sink.find_session_for_test(id)
}

trait Find {
    fn find_session_for_test(&self, id: &str) -> SessionHandle;
    fn msgs(&self, s: SessionHandle) -> Vec<&MessageDraft>;
}

impl Find for VecSink {
    fn find_session_for_test(&self, id: &str) -> SessionHandle {
        use vem_core::sink::ParseSink;
        self.find_session(id)
            .unwrap_or_else(|| panic!("no session {id}"))
    }
    fn msgs(&self, s: SessionHandle) -> Vec<&MessageDraft> {
        self.messages
            .iter()
            .filter(|(h, _, _)| *h == s)
            .map(|(_, _, m)| m)
            .collect()
    }
}

fn texts(m: &MessageDraft) -> Vec<String> {
    m.blocks.iter().filter_map(|b| b.text.clone()).collect()
}

/// The bubble messages of a session (not the composer's own `meta` record), as their `harness_uuid`.
fn bubble_ids(sink: &VecSink, s: SessionHandle) -> Vec<String> {
    sink.msgs(s)
        .iter()
        .filter(|m| m.harness_record_type == "bubble")
        .filter_map(|m| m.harness_uuid.clone())
        .collect()
}

#[test]
fn kv_session_fields() {
    let (sink, out) = global();
    assert!(matches!(out, ParseOutcome::Parsed { records } if records > 0));
    let s = session(&sink, C1);
    let d = sink.session_draft(s);
    assert_eq!(d.kind, SessionKind::Primary);
    assert_eq!(d.title.as_deref(), Some("list the files"));
    assert_eq!(d.harness_version.as_deref(), Some("composer-v3"));
    let first = d.first_ts.as_ref().unwrap();
    assert_eq!(first.value.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    assert_eq!(first.origin, TsOrigin::Stored);
    assert_eq!(
        d.last_ts.as_ref().unwrap().value.as_deref(),
        Some("2026-01-01T00:01:00.000Z")
    );
    assert!(sink
        .updates
        .iter()
        .any(|(h, u)| *h == s && u.model.as_deref() == Some("claude-4-sonnet")));
    let meta = sink.msgs(s)[0];
    assert_eq!(meta.role, Role::Meta);
    assert_eq!(meta.harness_record_type, "composerData");
    assert_eq!(meta.attributes["unifiedMode"], "agent");
    assert_eq!(meta.attributes["status"], "completed");
    assert_eq!(meta.attributes["context"], json!({"fileSelections": []}));
    let claims: Vec<_> = sink.claims.iter().filter(|(h, _)| *h == s).collect();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].1.scheme, "cursor:composerId");
    assert_eq!(claims[0].1.claimed_id, C1);
    let user = sink.msgs(s)[1];
    assert_eq!(user.role, Role::User);
    assert_eq!(texts(user), vec!["list the files"]);
    let asst = sink.msgs(s)[2];
    assert_eq!(asst.model.as_deref(), Some("claude-4-sonnet"));
    assert_eq!(asst.blocks[0].kind, BlockKind::Thinking);
    assert_eq!(asst.blocks[0].text.as_deref(), Some("look at the dir"));
    assert_eq!(asst.attributes["vem_turn"], format!("{C1}:0"));
    assert!(!asst.attributes.contains_key("text"));
    assert!(!asst.attributes.contains_key("thinking"));
}

#[test]
fn kv_bubble_order_follows_header() {
    let (sink, _) = global();
    let s = session(&sink, C1);
    // Header order b1, b2, b4 (b5 has no row), then the unreferenced b9.
    assert_eq!(bubble_ids(&sink, s), vec![b(1), b(2), b(4), b(9)]);
}

#[test]
fn kv_tool_call_absorbable() {
    let (sink, _) = global();
    let s = session(&sink, C1);
    let (_, _, tc) = sink
        .tool_calls
        .iter()
        .find(|(h, _, _)| *h == s)
        .expect("tool call");
    assert_eq!(tc.name, "run_terminal_cmd");
    assert_eq!(tc.category, ToolCategory::Shell);
    assert_eq!(tc.input, json!({"command": "ls"}));
    assert!(!tc.is_error);
    assert_eq!(tc.result_text.as_deref(), Some("a.txt\nb.txt"));
    let r = tc.tool_result.expect("result block");
    let (_, _, tool_msg) = sink
        .messages
        .iter()
        .find(|(_, h, _)| *h == r.message)
        .unwrap();
    assert_eq!(tool_msg.role, Role::Tool);
    assert_eq!(tool_msg.harness_record_type, "bubble.toolFormerData.result");
    assert!(tool_msg
        .blocks
        .iter()
        .all(|b| b.kind == BlockKind::ToolResult && b.tool_use_id.as_deref() == Some("call_1")));
    assert_eq!(tool_msg.attributes["vem_turn"], format!("{C1}:0"));
    let (_, _, use_msg) = sink
        .messages
        .iter()
        .find(|(_, h, _)| *h == tc.tool_use.message)
        .unwrap();
    let ub = &use_msg.blocks[tc.tool_use.ordinal as usize];
    assert_eq!(ub.kind, BlockKind::ToolUse);
    assert_eq!(ub.tool_use_id.as_deref(), Some("call_1"));
    assert_eq!(ub.payload["status"], "completed");
    // Same record: same locator and blob.
    assert_eq!(tool_msg.provenance, use_msg.provenance);
    let cmds = sink.observations_of(ObservationKind::CommandExecuted);
    assert_eq!(cmds[0].command.as_deref(), Some("ls"));
    assert_eq!(cmds[0].confidence, Confidence::High);
}

#[test]
fn kv_provenance_locator_and_blob() {
    let (sink, _) = global();
    let s = session(&sink, C1);
    let m = sink.msgs(s)[1];
    let p = &m.provenance;
    let suffix = " +wal"; // the fixture has a WAL, so every row is read from a WAL-applied view
    assert_eq!(
        p.locator.as_deref(),
        Some(format!("cursorDiskKV:bubbleId:{C1}:{}{suffix}", b(1)).as_str())
    );
    assert_eq!(p.byte_offset, 0);
    let blob = &sink.blobs[&p.content_sha256];
    assert_eq!(sha256_hex(blob), p.content_sha256);
    assert_eq!(p.byte_length, blob.len() as u64);
    assert_eq!(p.parser_name, "cursor-kv");
    assert_eq!(p.origin, ProvOrigin::Stored);
    let v: Value = serde_json::from_slice(blob).unwrap();
    assert_eq!(v["text"], "list the files");
    // One record index per row read (inline v2 bubbles share their composer row's index).
    let mut idx: Vec<u64> = sink
        .messages
        .iter()
        .filter(|(_, _, m)| {
            m.harness_record_type == "bubble"
                && m.provenance
                    .locator
                    .as_deref()
                    .unwrap()
                    .starts_with("cursorDiskKV:bubbleId:")
        })
        .map(|(_, _, m)| m.provenance.record_index)
        .collect();
    let n = idx.len();
    idx.sort();
    idx.dedup();
    assert_eq!(idx.len(), n);
}

#[test]
fn kv_composer_only_in_wal_is_parsed() {
    let (sink, _) = global();
    let s = session(&sink, C2);
    assert_eq!(sink.session_draft(s).title.as_deref(), Some("only in wal"));
    let m = sink.msgs(s);
    let user = m.iter().find(|m| m.role == Role::User).unwrap();
    assert_eq!(texts(user), vec!["newest prompt"]);
    assert!(user
        .provenance
        .locator
        .as_deref()
        .unwrap()
        .ends_with(" +wal"));
    let wal = sink.anomalies_of(AnomalyKind::WalApplied);
    assert_eq!(wal.len(), 1);
    assert_eq!(wal[0].severity, Severity::Info);
    let wal_bytes =
        std::fs::read(fixture("ide").join("User/globalStorage/state.vscdb-wal")).unwrap();
    assert_eq!(wal[0].details["wal_sha256"], sha256_hex(&wal_bytes));
}

#[test]
fn unreferenced_bubble_kept() {
    let (sink, _) = global();
    let s = session(&sink, C1);
    let m = sink
        .msgs(s)
        .into_iter()
        .find(|m| m.harness_uuid.as_deref() == Some(&b(9)))
        .expect("unreferenced bubble kept");
    assert_eq!(texts(m), vec!["a deleted reply"]);
    let ur = sink.anomalies_of(AnomalyKind::UnreferencedRecord);
    assert_eq!(ur.len(), 1, "{ur:?}");
    assert_eq!(ur[0].severity, Severity::Info);
    assert_eq!(ur[0].details, json!({"composer": C1, "bubble": b(9)}));
    assert_eq!(ur[0].session, Some(s));
    assert_eq!(ur[0].provenance.as_ref(), Some(&m.provenance));
    // Not a separate turn: it continues the last turn of the composer.
    assert_eq!(m.attributes["vem_turn"], format!("{C1}:0"));
}

#[test]
fn header_without_bubble_reported() {
    let (sink, _) = global();
    let s = session(&sink, C1);
    let mt: Vec<_> = sink
        .anomalies_of(AnomalyKind::MissingTranscript)
        .into_iter()
        .filter(|a| a.details["composer"] == C1)
        .collect();
    assert_eq!(mt.len(), 1, "{mt:?}");
    assert_eq!(mt[0].severity, Severity::Info);
    assert_eq!(mt[0].details, json!({"composer": C1, "bubble": b(5)}));
    assert_eq!(mt[0].session, Some(s));
    // C4's header lists b41..b43 and all three rows exist (one is malformed): nothing missing there.
    assert!(sink
        .anomalies_of(AnomalyKind::MissingTranscript)
        .iter()
        .all(|a| a.details["composer"] != C4));
}

#[test]
fn orphan_bubble_composer_is_sidecar_only() {
    let (sink, _) = global();
    let s = session(&sink, C9);
    assert_eq!(sink.session_draft(s).kind, SessionKind::SidecarOnly);
    assert_eq!(bubble_ids(&sink, s), vec![b(91)]);
    let mt: Vec<_> = sink
        .anomalies_of(AnomalyKind::MissingTranscript)
        .into_iter()
        .filter(|a| a.details["composer"] == C9)
        .collect();
    assert_eq!(mt.len(), 1);
    assert_eq!(mt[0].severity, Severity::Warning);
    assert_eq!(mt[0].session, Some(s));
    assert!(sink
        .claims
        .iter()
        .any(|(h, c)| *h == s && c.claimed_id == C9));
}

#[test]
fn inline_conversation_v2_parsed() {
    let (sink, _) = global();
    let s = session(&sink, C3);
    assert_eq!(bubble_ids(&sink, s), vec![b(31), b(32)]);
    let m = sink.msgs(s);
    let composer_sha = &m[0].provenance.content_sha256;
    let user = m
        .iter()
        .find(|m| m.harness_uuid.as_deref() == Some(&b(31)))
        .unwrap();
    assert_eq!(
        user.provenance.locator.as_deref(),
        Some(format!("cursorDiskKV:composerData:{C3}#conversation[0] +wal").as_str())
    );
    assert_eq!(&user.provenance.content_sha256, composer_sha);
    assert_eq!(texts(user), vec!["hi"]);
    let asst = m
        .iter()
        .find(|m| m.harness_uuid.as_deref() == Some(&b(32)))
        .unwrap();
    assert_eq!(asst.attributes["vem_turn"], format!("{C3}:0"));
}

#[test]
fn unknown_prefix_census_one_anomaly_per_prefix() {
    let (sink, _) = global();
    let census: Vec<_> = sink
        .anomalies_of(AnomalyKind::UnknownRecordType)
        .into_iter()
        .filter(|a| a.details.get("prefix").is_some())
        .collect();
    let by = |p: &str| {
        let v: Vec<_> = census.iter().filter(|a| a.details["prefix"] == p).collect();
        assert_eq!(v.len(), 1, "{p}: {census:?}");
        v[0].details.clone()
    };
    assert_eq!(
        by("checkpointId"),
        json!({"prefix": "checkpointId", "count": 1, "parsed": false})
    );
    assert_eq!(
        by("codeBlockDiff"),
        json!({"prefix": "codeBlockDiff", "count": 2, "parsed": false})
    );
    assert_eq!(
        by("weirdPrefix"),
        json!({"prefix": "weirdPrefix", "count": 1, "parsed": false, "known": false})
    );
    assert_eq!(census.len(), 3);
}

#[test]
fn oversized_value_skipped() {
    let mut sink = VecSink::default();
    let root = fixture("ide");
    sink.live_root = Some(root.clone());
    let out = with_ctx(&CursorIdeAdapter, &root, DB, 1, |ctx| {
        // Larger than the short bubbles, smaller than C1's composer row.
        parse_state_vscdb_with_cap(ctx, &mut sink, 400).unwrap()
    });
    assert!(matches!(out, ParseOutcome::Parsed { .. }));
    let big = sink.anomalies_of(AnomalyKind::OversizedRecord);
    let c1 = big
        .iter()
        .find(|a| a.details["key"] == format!("composerData:{C1}"))
        .expect("C1 composer skipped");
    assert!(c1.details["length"].as_u64().unwrap() > 400);
    assert!(c1.provenance.is_none());
    // The bubbles are still read, and still belong to the composer.
    let s = session(&sink, C1);
    assert!(bubble_ids(&sink, s).contains(&b(1)));
}

#[test]
fn non_json_value_malformed_not_fatal() {
    let (sink, _) = global();
    let s = session(&sink, C4);
    let bad: Vec<_> = sink
        .anomalies_of(AnomalyKind::MalformedRecord)
        .into_iter()
        .filter(|a| {
            a.provenance
                .as_ref()
                .and_then(|p| p.locator.as_deref())
                .is_some_and(|l| l.contains(&b(41)))
        })
        .collect();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].session, Some(s));
    let p = bad[0].provenance.as_ref().unwrap();
    assert_eq!(sink.blobs[&p.content_sha256], b"this is not json {");
    assert_eq!(bubble_ids(&sink, s), vec![b(42), b(43)]);
}

#[test]
fn itemtable_legacy_inventoried() {
    let mut sink = VecSink::default();
    let out = parse_ide(&fixture("vscode"), DB, 1, &mut sink);
    assert_eq!(out, ParseOutcome::NotParsed);
    let g = sink.anomalies_of(AnomalyKind::UnknownStoreGeneration);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].severity, Severity::Info);
    assert!(sink.sessions.is_empty());
}

#[test]
fn wal_and_shm_not_parsed() {
    let mut sink = VecSink::default();
    let out = parse_ide(
        &fixture("ide"),
        "User/globalStorage/state.vscdb-wal",
        1,
        &mut sink,
    );
    assert_eq!(out, ParseOutcome::NotParsed);
}

#[test]
fn created_at_string_and_number_both_stored() {
    let (sink, _) = global();
    let s = session(&sink, C4);
    let m = sink.msgs(s);
    let ts = |id: &str| {
        m.iter()
            .find(|m| m.harness_uuid.as_deref() == Some(id))
            .unwrap()
            .timestamp
            .clone()
    };
    let t42 = ts(&b(42));
    assert_eq!(t42.value.as_deref(), Some("2025-12-31T23:00:05.000Z"));
    assert_eq!(t42.origin, TsOrigin::Stored);
    let t43 = ts(&b(43));
    assert_eq!(t43.value.as_deref(), Some("2025-12-31T23:56:45.000Z"));
    assert_eq!(t43.origin, TsOrigin::Stored);
    assert!(sink
        .anomalies_of(AnomalyKind::MissingTimestamp)
        .iter()
        .all(|a| a.details["composer"] != C4));
}

#[test]
fn missing_created_at_aggregated_per_composer() {
    let (sink, _) = global();
    // C2's bubble (only in the WAL) has no createdAt.
    let s = session(&sink, C2);
    let mt: Vec<_> = sink
        .anomalies_of(AnomalyKind::MissingTimestamp)
        .into_iter()
        .filter(|a| a.session == Some(s))
        .collect();
    assert_eq!(mt.len(), 1);
    assert_eq!(mt[0].details["count"], 1);
    assert_eq!(
        sink.msgs(s).last().unwrap().timestamp.origin,
        TsOrigin::Absent
    );
}

/// `cargo test -p vem-adapters --test cursor_kv bounded_memory_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn bounded_memory_smoke() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("User/globalStorage");
    std::fs::create_dir_all(&dir).unwrap();
    let conn = Connection::open(dir.join("state.vscdb")).unwrap();
    conn.execute_batch(
        "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
         CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
         BEGIN;",
    )
    .unwrap();
    let composers = 20;
    let per = 1000;
    // The VecSink keeps every message and blob, so the filler stays small; the parser streams either way.
    let filler = "x".repeat(500);
    for c in 0..composers {
        let cid = format!("c{c:04}");
        let headers: Vec<Value> = (0..per)
            .map(|i| json!({"bubbleId": format!("b{i:06}"), "type": 1 + i % 2}))
            .collect();
        conn.execute(
            "INSERT INTO cursorDiskKV VALUES (?1, ?2)",
            (
                format!("composerData:{cid}"),
                json!({"_v": 3, "composerId": cid, "createdAt": 1767225600000u64, "fullConversationHeadersOnly": headers}).to_string(),
            ),
        )
        .unwrap();
        for i in 0..per {
            conn.execute(
                "INSERT INTO cursorDiskKV VALUES (?1, ?2)",
                (
                    format!("bubbleId:{cid}:b{i:06}"),
                    json!({"type": 1 + i % 2, "text": filler, "createdAt": 1767225600000u64 + i as u64}).to_string(),
                ),
            )
            .unwrap();
        }
    }
    conn.execute_batch("COMMIT;").unwrap();
    drop(conn);
    let mut sink = VecSink::default();
    let out = parse_ide(tmp.path(), DB, 1, &mut sink);
    assert_eq!(
        out,
        ParseOutcome::Parsed {
            records: (composers * (per + 1)) as u64
        }
    );
    assert_eq!(sink.messages.len(), (composers * (per + 1)) as usize);
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let hwm = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    println!(
        "bounded_memory_smoke: {} bubbles, peak RSS (VmHWM) {} MiB",
        composers * per,
        hwm / 1024
    );
    assert!(hwm < 200 * 1024, "peak RSS {hwm} KiB");
}
