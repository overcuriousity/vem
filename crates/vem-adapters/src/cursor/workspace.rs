//! Cursor workspace storage: `User/workspaceStorage/<h>/state.vscdb` lists the composers opened in a
//! workspace (`ItemTable` key `composer.composerData`), and `workspace.json` names its folder. The
//! folder becomes the `project_path` of those composer sessions. The folder URI is only decoded to a
//! string, never opened.

use super::record_anomaly;
use super::sqlite::{has_table, open_for_parse, RECORD_CAP};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::path::Path;
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "cursor-workspace";
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");

const KEY: &str = "composer.composerData";

/// The path of a `file://` URI, percent-decoded (`file:///c%3A/x` is `c:/x`). Any other URI is kept as
/// given.
pub fn file_uri_path(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_string();
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8_lossy(&out).into_owned();
    // `/c:/x`: a Windows drive path.
    match path.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() => path[1..].to_string(),
        _ => path,
    }
}

/// The workspace folder from `workspace.json` next to the db (`folder`, or `workspace` for a
/// multi-root workspace file).
fn folder(ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Option<String> {
    let rel = ctx.rel_path.parent()?.join("workspace.json");
    let bytes = sink.read_root_file(&rel, RECORD_CAP).ok()??;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    ["folder", "workspace"]
        .iter()
        .find_map(|k| v.get(k).and_then(Value::as_str))
        .map(file_uri_path)
}

pub fn parse_workspace_file(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    if ctx.rel_path.file_name() != Some("state.vscdb".as_ref()) {
        // `workspace.json` is read by the db's parser; `-wal`/`-shm` are applied with the db.
        return Ok(ParseOutcome::NotParsed);
    }
    let workspace = ctx
        .rel_path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let view = open_for_parse(ctx, sink)?;
    if !has_table(&view.conn, "ItemTable") {
        return Ok(ParseOutcome::NotParsed);
    }
    let sql = |e: rusqlite::Error| ParseError::Invalid(format!("sqlite: {e}"));
    let Some(len) = view
        .conn
        .query_row(
            "SELECT octet_length(value) FROM ItemTable WHERE key = ?1",
            [KEY],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(sql)?
    else {
        return Ok(ParseOutcome::Parsed { records: 0 });
    };
    let len = len.unwrap_or(0).max(0) as u64;
    if len > RECORD_CAP {
        sink.anomaly(record_anomaly(
            ctx.handle,
            None,
            AnomalyKind::OversizedRecord,
            Severity::Warning,
            None,
            format!("ItemTable {KEY} is {len} bytes, over the cap; skipped"),
            json!({ "key": KEY, "length": len }),
        ));
        return Ok(ParseOutcome::Parsed { records: 0 });
    }
    let bytes: Vec<u8> = view
        .conn
        .query_row(
            "SELECT CAST(value AS BLOB) FROM ItemTable WHERE key = ?1",
            [KEY],
            |r| Ok(r.get::<_, Option<Vec<u8>>>(0)?.unwrap_or_default()),
        )
        .map_err(sql)?;
    let prov = Provenance {
        source_file: ctx.handle,
        byte_offset: 0,
        byte_length: bytes.len() as u64,
        record_index: 0,
        content_sha256: sink.blob(&bytes),
        parser_name: PARSER_NAME.into(),
        parser_version: PARSER_VERSION.into(),
        origin: ProvOrigin::Stored,
        locator: Some(format!(
            "ItemTable:{KEY}{}",
            if view.wal_applied { " +wal" } else { "" }
        )),
    };
    let v: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            sink.anomaly(record_anomaly(
                ctx.handle,
                None,
                AnomalyKind::MalformedRecord,
                Severity::Error,
                Some(&prov),
                format!("{KEY} is not JSON: {e}"),
                json!({ "workspace": workspace }),
            ));
            return Ok(ParseOutcome::Parsed { records: 1 });
        }
    };
    let project = folder(ctx, sink);
    let ids = v
        .get("allComposers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("composerId").and_then(Value::as_str));
    for id in ids {
        match sink.find_session(id) {
            Some(s) => {
                if let Some(p) = &project {
                    sink.update_session(
                        s,
                        SessionUpdate {
                            project_path: Some(p.clone()),
                            ..Default::default()
                        },
                    );
                }
            }
            None => sink.anomaly(record_anomaly(
                ctx.handle,
                None,
                AnomalyKind::MissingTranscript,
                Severity::Info,
                Some(&prov),
                format!("workspace {workspace} lists composer {id}, which has no session"),
                json!({ "composer": id, "workspace": workspace }),
            )),
        }
    }
    Ok(ParseOutcome::Parsed { records: 1 })
}
