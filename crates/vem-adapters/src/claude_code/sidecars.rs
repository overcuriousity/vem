//! Sidecar stores of a `.claude` directory: `history.jsonl` (prompt history with pasted content).

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;
use vem_core::adapter::{FileContext, ParseError};
use vem_core::hash::sha256_hex;
use vem_core::jsonl::JsonlReader;
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const HISTORY_PARSER_NAME: &str = "claude_code.history";
pub const HISTORY_PARSER_VERSION: &str = "1";

fn epoch_ms(v: Option<&Value>) -> Timestamp {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).and_then(Timestamp::stored_epoch_ms),
        Some(Value::String(s)) => s.parse::<i64>().ok().and_then(Timestamp::stored_epoch_ms).or_else(|| Timestamp::stored(s)),
        _ => None,
    }
    .unwrap_or_else(Timestamp::absent)
}

/// Each `history.jsonl` line is one submitted prompt: `display`, `pastedContents`, `timestamp` (ms), `project`, `sessionId`.
/// Pasted content becomes a `paste_detected` observation; a session id with no transcript is a `missing_transcript` anomaly.
pub fn parse_history(ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
    let file = File::open(&ctx.abs_path)?;
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for rec in JsonlReader::new(BufReader::new(file)) {
        let rec = rec?;
        let prov = Provenance {
            source_file: ctx.handle,
            byte_offset: rec.offset,
            byte_length: rec.length,
            record_index: rec.index,
            content_sha256: sha256_hex(&rec.bytes),
            parser_name: HISTORY_PARSER_NAME.to_string(),
            parser_version: HISTORY_PARSER_VERSION.to_string(),
            origin: ProvOrigin::Stored,
        };
        let v: Value = match serde_json::from_slice(&rec.bytes) {
            Ok(v) => v,
            Err(e) => {
                sink.anomaly(AnomalyDraft {
                    kind: if rec.terminated { AnomalyKind::MalformedRecord } else { AnomalyKind::TruncatedLine },
                    severity: if rec.terminated { Severity::Error } else { Severity::Warning },
                    source_file: Some(ctx.handle),
                    session: None,
                    byte_offset: Some(rec.offset),
                    message: format!("history.jsonl line {} is not valid JSON: {e}", rec.index),
                    details: json!({}),
                });
                continue;
            }
        };
        let session_id = v.get("sessionId").and_then(Value::as_str).unwrap_or("").to_string();
        let timestamp = epoch_ms(v.get("timestamp"));
        let pasted = v.get("pastedContents").and_then(Value::as_object).map(|m| !m.is_empty()).unwrap_or(false);
        match sink.find_session(&session_id) {
            Some(session) => {
                if pasted {
                    sink.observation(
                        session,
                        ObservationDraft {
                            kind: ObservationKind::PasteDetected,
                            derived_from: Derivation::Record(prov),
                            path: None,
                            command: None,
                            before_blob: None,
                            after_blob: None,
                            timestamp,
                            confidence: Confidence::High,
                            details: json!({
                                "display": v.get("display").cloned().unwrap_or(Value::Null),
                                "pastedContents": v.get("pastedContents").cloned().unwrap_or(Value::Null),
                                "project": v.get("project").cloned().unwrap_or(Value::Null),
                            }),
                        },
                    );
                }
            }
            None => {
                if !session_id.is_empty() && missing.insert(session_id.clone()) {
                    sink.anomaly(AnomalyDraft {
                        kind: AnomalyKind::MissingTranscript,
                        severity: Severity::Warning,
                        source_file: Some(ctx.handle),
                        session: None,
                        byte_offset: Some(rec.offset),
                        message: format!("history.jsonl references session {session_id} but no transcript exists in this root"),
                        details: json!({
                            "sessionId": session_id,
                            "project": v.get("project").cloned().unwrap_or(Value::Null),
                            "display": v.get("display").cloned().unwrap_or(Value::Null),
                            "timestamp": timestamp.value,
                        }),
                    });
                }
            }
        }
    }
    Ok(())
}
