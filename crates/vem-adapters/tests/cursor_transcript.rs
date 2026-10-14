//! Cursor agent transcripts (`agent-transcripts/**/*.jsonl`) and the Cursor tool vocabulary.

mod cursor_common;
use cursor_common::*;
use serde_json::json;
use vem_adapters::cursor::tools::{categorize, derive_observations};
use vem_core::adapter::ParseOutcome;
use vem_core::model::*;
use vem_core::testing::VecSink;

fn t1() -> VecSink {
    let mut sink = VecSink::default();
    parse_cursor(&fixture("dotcursor"), T1_FILE, &mut sink);
    sink
}

#[test]
fn t1_messages() {
    let mut sink = t1();
    // The parser version is the crate version; keep the snapshot stable across releases.
    for (_, _, m) in &mut sink.messages {
        assert_eq!(m.provenance.parser_version, env!("CARGO_PKG_VERSION"));
        m.provenance.parser_version = "<version>".into();
    }
    insta::assert_json_snapshot!("t1_messages", (&sink.messages, &sink.claims));
}

#[test]
fn transcript_timestamps_absent_and_bounds_from_mtime() {
    let sink = t1();
    let (_, s) = &sink.sessions[0];
    assert_eq!(s.harness_session_id, T1);
    assert_eq!(s.kind, SessionKind::Primary);
    assert_eq!(s.project_path.as_deref(), Some("home-alice-proj"));
    assert_eq!(s.first_ts.as_ref().unwrap().origin, TsOrigin::Absent);
    assert_eq!(s.last_ts.as_ref().unwrap().origin, TsOrigin::FileMtime);
    assert!(s.last_ts.as_ref().unwrap().value.is_some());
    assert!(sink
        .messages
        .iter()
        .all(|(_, _, m)| m.timestamp.origin == TsOrigin::Absent));
    assert!(sink.anomalies_of(AnomalyKind::MissingTimestamp).is_empty());
    for (_, _, m) in &sink.messages {
        assert_eq!(m.provenance.parser_name, "cursor-transcript");
        assert!(m.provenance.locator.is_none());
    }
}

#[test]
fn assistant_records_share_vem_turn_until_next_user() {
    let body = [
        r#"{"role":"user","message":{"content":"a"}}"#,
        r#"{"role":"assistant","message":{"content":"x"}}"#,
        r#"{"role":"assistant","message":{"content":"y"}}"#,
        r#"{"role":"user","message":{"content":"b"}}"#,
        r#"{"role":"assistant","message":{"content":"z"}}"#,
    ]
    .join("\n")
        + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let turns: Vec<Option<&str>> = sink
        .messages
        .iter()
        .map(|(_, _, m)| m.attributes.get("vem_turn").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        turns,
        vec![None, Some("s:0"), Some("s:0"), None, Some("s:3")]
    );
}

#[test]
fn subagent_linked_to_parent() {
    let mut sink = VecSink::default();
    parse_cursor(&fixture("dotcursor"), SUB_FILE, &mut sink);
    let (_, s) = &sink.sessions[0];
    assert_eq!(s.harness_session_id, "1c1c1c1c-0000-4000-8000-0000000000a1");
    assert_eq!(s.kind, SessionKind::Subagent);
    assert_eq!(s.parent_harness_session_id.as_deref(), Some(T1));
    assert_eq!(
        sink.claims[0].1.claimed_id,
        "1c1c1c1c-0000-4000-8000-0000000000a1"
    );
}

#[test]
fn tool_use_without_result_is_resultless_call() {
    let sink = t1();
    assert_eq!(sink.tool_calls.len(), 1);
    let (_, _, tc) = &sink.tool_calls[0];
    assert_eq!(tc.name, "Shell");
    assert_eq!(tc.category, ToolCategory::Shell);
    assert_eq!(tc.input, json!({"command": "ls"}));
    assert!(tc.tool_result.is_none());
    assert!(tc.result_text.is_none());
    let cmds = sink.observations_of(ObservationKind::CommandExecuted);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0].command.as_deref(), Some("ls"));
    assert_eq!(cmds[0].confidence, Confidence::Medium);
}

#[test]
fn tool_result_pairs_when_present() {
    let body = [
        r#"{"role":"user","message":{"content":"go"}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"read_file","input":{"target_file":"a.rs"}}]}}"#,
        r#"{"role":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"fn main(){}"}]}}"#,
    ]
    .join("\n")
        + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.tool_calls.len(), 1);
    let (_, _, tc) = &sink.tool_calls[0];
    assert_eq!(tc.category, ToolCategory::FileRead);
    let r = tc.tool_result.expect("paired");
    assert_eq!(r.message, sink.messages[2].1);
    assert_eq!(tc.result_text.as_deref(), Some("fn main(){}"));
    let reads = sink.observations_of(ObservationKind::FileRead);
    assert_eq!(reads[0].path.as_deref(), Some("a.rs"));
    assert_eq!(reads[0].confidence, Confidence::High);
}

#[test]
fn agent_tools_sidecar_attached_by_call_id() {
    let sink = t1();
    let (_, _, tc) = &sink.tool_calls[0];
    let sha = tc.result_payload.as_ref().unwrap()["sidecar_blob"]
        .as_str()
        .unwrap()
        .to_string();
    let expected = std::fs::read(fixture("dotcursor").join(
        "projects/home-alice-proj/agent-transcripts/1c1c1c1c-0000-4000-8000-000000000001/agent-tools/toolu_cursor_01.txt",
    ))
    .unwrap();
    assert_eq!(sink.blobs[&sha], expected);
    assert!(tc.tool_result.is_none(), "a sidecar is not a tool_result");
}

#[test]
fn sidecar_name_with_dotdot_is_suspicious() {
    let body = [
        r#"{"role":"user","message":{"content":"go"}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"tool_use","id":"../x","name":"Shell","input":{"command":"id"}}]}}"#,
    ]
    .join("\n")
        + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    // A file the traversal would reach if it were followed.
    std::fs::write(
        tmp.path()
            .join("projects/home-bob-x/agent-transcripts/s/x.txt"),
        "secret",
    )
    .unwrap();
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let sus = sink.anomalies_of(AnomalyKind::SuspiciousPath);
    assert_eq!(sus.len(), 1);
    assert_eq!(sus[0].details["tool_use_id"], "../x");
    assert!(sink.tool_calls[0].2.result_payload.is_none());
    assert!(sink.blobs.values().all(|b| b != b"secret"));
}

#[test]
fn unknown_field_aggregated_once() {
    let body = [
        r#"{"role":"user","foo":1,"message":{"content":"a"}}"#,
        r#"{"role":"assistant","foo":2,"message":{"content":"b","bar":true}}"#,
        r#"{"role":"assistant","foo":3,"message":{"content":"c"}}"#,
    ]
    .join("\n")
        + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let uf = sink.anomalies_of(AnomalyKind::UnknownField);
    assert_eq!(uf.len(), 2, "{uf:?}");
    let foo = uf.iter().find(|a| a.details["field"] == "foo").unwrap();
    assert_eq!(foo.details["count"], 3);
    assert_eq!(foo.details["record_kind"], "line");
    assert_eq!(foo.details["first_locator"], "line 0");
    assert_eq!(foo.severity, Severity::Info);
    let bar = uf.iter().find(|a| a.details["field"] == "bar").unwrap();
    assert_eq!(bar.details["record_kind"], "message");
    assert_eq!(bar.details["count"], 1);
    // Unknown fields are kept, not dropped.
    assert_eq!(sink.messages[0].2.attributes["foo"], 1);
}

#[test]
fn unknown_block_types_aggregated_once_per_type() {
    let body = [
        r#"{"role":"user","message":{"content":[{"type":"text","text":"a"}]}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"text","text":"b"},{"type":"tool_call","x":1}]}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"tool_call","x":2},{"type":"widget"},{"no_type":true},"bare"]}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Shell","input":{}}]}}"#,
    ]
    .join("\n")
        + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let ub: Vec<_> = sink
        .anomalies_of(AnomalyKind::UnknownRecordType)
        .into_iter()
        .filter(|a| a.details["record_kind"] == "content_block")
        .collect();
    assert_eq!(ub.len(), 4, "{ub:?}");
    let of = |t: &str| ub.iter().find(|a| a.details["block_type"] == t).unwrap();
    assert_eq!(of("tool_call").details["reason"], "unknown_type");
    let tc = of("tool_call");
    assert_eq!(tc.details["count"], 2);
    assert_eq!(tc.details["first_locator"], "line 1 block 1");
    assert_eq!(tc.severity, Severity::Warning);
    assert_eq!(tc.provenance.as_ref().unwrap().record_index, 1);
    assert_eq!(of("widget").details["count"], 1);
    // A block without a string `type` is counted too, never dropped from the census.
    let untyped = of("(no type)");
    assert_eq!(untyped.details["count"], 2);
    assert_eq!(untyped.details["first_locator"], "line 2 block 2");
    // A tool_use without an id cannot be paired; that is reported, not skipped silently.
    let no_id = of("tool_use");
    assert_eq!(no_id.details["reason"], "missing_id");
    assert_eq!(no_id.details["first_locator"], "line 3 block 0");
    // The blocks themselves are kept on the message as `other`.
    assert_eq!(sink.messages[2].2.blocks.len(), 4);
}

#[test]
fn unknown_role_is_meta() {
    let body = r#"{"role":"system","message":{"content":"x"}}"#.to_string() + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages[0].2.role, Role::Meta);
    assert_eq!(sink.anomalies_of(AnomalyKind::UnknownRecordType).len(), 1);
}

#[test]
fn truncated_last_line_is_truncated_line() {
    let body = r#"{"role":"user","message":{"content":"a"}}
{"role":"assistant","message":{"con"#;
    let (tmp, rel) = temp_transcript("s", body);
    let mut sink = VecSink::default();
    let out = parse_cursor(tmp.path(), &rel, &mut sink);
    assert_eq!(out, ParseOutcome::Parsed { records: 2 });
    assert_eq!(sink.messages.len(), 1);
    let t = sink.anomalies_of(AnomalyKind::TruncatedLine);
    assert_eq!(t.len(), 1);
    assert!(t[0].provenance.is_some());
}

#[test]
fn txt_transcript_not_parsed() {
    let mut sink = VecSink::default();
    let out = parse_cursor(
        &fixture("dotcursor"),
        "projects/home-alice-proj/agent-transcripts/1c1c1c1c-0000-4000-8000-000000000000.txt",
        &mut sink,
    );
    assert_eq!(out, ParseOutcome::NotParsed);
}

#[test]
fn categorize_table() {
    use ToolCategory::*;
    let rows: &[(&str, ToolCategory)] = &[
        ("Shell", Shell),
        ("run_terminal_cmd", Shell),
        ("run_terminal_command", Shell),
        ("run_terminal_cmd_v2", Shell),
        ("Read", FileRead),
        ("read_file", FileRead),
        ("ReadLints", FileRead),
        ("read_file_v3", FileRead),
        ("Write", FileWrite),
        ("write", FileWrite),
        ("create_file", FileWrite),
        ("StrReplace", FileEdit),
        ("search_replace", FileEdit),
        ("edit_file", FileEdit),
        ("MultiEdit", FileEdit),
        ("EditNotebook", FileEdit),
        ("apply_patch", FileEdit),
        ("Delete", FileEdit),
        ("delete_file", FileEdit),
        ("Glob", Search),
        ("Grep", Search),
        ("LS", Search),
        ("list_dir", Search),
        ("grep_search", Search),
        ("file_search", Search),
        ("codebase_search", Search),
        ("SemanticSearch", Search),
        ("glob_file_search", Search),
        ("WebSearch", Web),
        ("WebFetch", Web),
        ("web_search", Web),
        ("fetch_url", Web),
        ("Task", Agent),
        ("mcp_github_x", Mcp),
        ("mcp__srv__tool", Mcp),
        ("something_else", Other),
        ("_v2", Other),
    ];
    for (name, want) in rows {
        assert_eq!(categorize(name), *want, "{name}");
    }
}

#[test]
fn derive_observations_table() {
    let tc = ToolCallHandle(7);
    let at = Timestamp::absent();
    let one = |name: &str, input: serde_json::Value| {
        let mut v = derive_observations(name, &input, Some(true), tc, &at);
        assert_eq!(v.len(), 1, "{name}");
        v.remove(0)
    };
    let o = one("run_terminal_cmd", json!({"command": "ls -la"}));
    assert_eq!(
        (o.kind, o.command.as_deref()),
        (ObservationKind::CommandExecuted, Some("ls -la"))
    );
    assert_eq!(o.confidence, Confidence::High);
    assert_eq!(o.derived_from, Derivation::ToolCall(tc));
    let o = one("read_file", json!({"relative_workspace_path": "r.txt"}));
    assert_eq!(
        (o.kind, o.path.as_deref()),
        (ObservationKind::FileRead, Some("r.txt"))
    );
    let o = one("Read", json!({"path": "p", "file_path": "f"}));
    assert_eq!(o.path.as_deref(), Some("p"));
    let o = one("Write", json!({"file_path": "w"}));
    assert_eq!(
        (o.kind, o.path.as_deref()),
        (ObservationKind::FileWritten, Some("w"))
    );
    let o = one("edit_file", json!({"target_file": "e"}));
    assert_eq!(
        (o.kind, o.path.as_deref()),
        (ObservationKind::FileEdited, Some("e"))
    );
    let o = one("delete_file", json!({"target_file": "d"}));
    assert_eq!(
        (o.kind, o.path.as_deref()),
        (ObservationKind::FileDeleted, Some("d"))
    );
    let o = one("Delete", json!({"path": "d2"}));
    assert_eq!(o.kind, ObservationKind::FileDeleted);
    let o = one("WebFetch", json!({"url": "https://x"}));
    assert_eq!(
        (o.kind, o.path.as_deref()),
        (ObservationKind::UrlReferenced, Some("https://x"))
    );
    let o = one("web_search", json!({"query": "q"}));
    assert_eq!(
        (o.kind, o.path.as_deref()),
        (ObservationKind::UrlReferenced, Some("q"))
    );
    let o = one("Task", json!({}));
    assert_eq!(o.kind, ObservationKind::SubagentSpawned);
    assert!(derive_observations("Grep", &json!({}), None, tc, &at).is_empty());
    let o = derive_observations("Shell", &json!({"command": "x"}), Some(false), tc, &at);
    assert_eq!(o[0].confidence, Confidence::Medium);
}

/// The shape of real Cursor agent transcripts (MIK-213, from the owner's structure-only report): tool
/// calls are `tool_use` blocks with `name` and `input` but no `id`, and no `tool_result` blocks follow.
#[test]
fn id_less_tool_use_blocks_are_tool_calls() {
    let body = [
        r#"{"role":"user","message":{"content":[{"type":"text","text":"q"}]}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"text","text":"a"},{"type":"tool_use","name":"Shell","input":{"command":"ls","description":"d"}}]}}"#,
        r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"StrReplace","input":{"path":"/p/f.rs","old_string":"a","new_string":"b"}},{"type":"tool_use","name":"Read","input":{"path":"/p/g.rs","limit":10,"offset":5}}]}}"#,
        r#"{"type":"turn_ended","status":"ok"}"#,
    ]
    .join("\n")
        + "\n";
    let (tmp, rel) = temp_transcript("s", &body);
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let calls: Vec<_> = sink.tool_calls.iter().map(|(_, _, t)| t).collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_eq!(calls[0].name, "Shell");
    assert_eq!(calls[0].category, ToolCategory::Shell);
    assert_eq!(calls[0].input["command"], "ls");
    assert_eq!(calls[0].tool_use.ordinal, 1);
    assert_eq!(calls[1].name, "StrReplace");
    assert_eq!(calls[2].name, "Read");
    assert_eq!(calls[2].tool_use.ordinal, 1);
    assert!(calls
        .iter()
        .all(|c| c.tool_result.is_none() && c.result_payload.is_none()));
    // The missing id is still reported once, as information: the calls are parsed but cannot be paired.
    let no_id: Vec<_> = sink
        .anomalies_of(AnomalyKind::UnknownRecordType)
        .into_iter()
        .filter(|a| a.details["block_type"] == "tool_use")
        .collect();
    assert_eq!(no_id.len(), 1, "{no_id:?}");
    assert_eq!(no_id[0].details["count"], 3);
    assert_eq!(no_id[0].details["reason"], "missing_id");
    assert_eq!(no_id[0].severity, Severity::Info);
}

fn user_query_of(text: &str) -> Option<serde_json::Value> {
    let line = json!({"role": "user", "message": {"content": [{"type": "text", "text": text}]}});
    let (tmp, rel) = temp_transcript("q", &format!("{line}\n"));
    let mut sink = VecSink::default();
    parse_cursor(tmp.path(), &rel, &mut sink);
    let (_, _, m) = &sink.messages[0];
    // The block keeps the text verbatim; only the computed attribute unwraps it.
    assert_eq!(m.blocks[0].text.as_deref(), Some(text));
    assert_eq!(m.timestamp.origin, TsOrigin::Absent);
    m.attributes.get("vem_user_query").cloned()
}

#[test]
fn user_query_wrapper_unwrapped_with_embedded_timestamp() {
    assert_eq!(
        user_query_of(
            "<timestamp>Tuesday, Aug 18, 2026, 9:38 PM (UTC+3)</timestamp>\n<user_query>\nhello there\n</user_query>"
        ),
        Some(json!({
            "text": "hello there",
            "embedded_timestamp": "Tuesday, Aug 18, 2026, 9:38 PM (UTC+3)",
            "embedded_timestamp_utc": "2026-08-18T18:38:00.000Z",
        }))
    );
    assert_eq!(
        user_query_of("<timestamp>Monday, January 1, 2024, 12:05 AM (UTC-5:30)</timestamp><user_query>x</user_query>")
            .unwrap()["embedded_timestamp_utc"],
        json!("2024-01-01T05:35:00.000Z")
    );
    // An unrecognised timestamp format stays text, without a normalised value.
    assert_eq!(
        user_query_of("<timestamp>yesterday-ish</timestamp>\n<user_query>q</user_query>"),
        Some(json!({ "text": "q", "embedded_timestamp": "yesterday-ish" }))
    );
    // A query without a timestamp.
    assert_eq!(
        user_query_of("<user_query>\nonly\n</user_query>\n"),
        Some(json!({ "text": "only" }))
    );
}

#[test]
fn user_query_malformed_wrapper_kept_verbatim() {
    for text in [
        "plain prompt",
        "<user_query>no close",
        "<timestamp>t</timestamp> no query",
        "<timestamp>t<user_query>q</user_query>",
        "before <user_query>q</user_query>",
        "<user_query>q</user_query> after",
    ] {
        assert_eq!(user_query_of(text), None, "{text:?}");
    }
}
