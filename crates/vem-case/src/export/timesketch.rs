//! Timesketch / Vestigo CSV and JSONL writers (spec §9; Vestigo `INPUT_FORMATS.md`).

use super::Event;
use crate::error::CaseError;
use std::collections::BTreeSet;
use std::io::Write;

const FIXED: [&str; 7] = ["datetime", "timestamp_desc", "message", "source", "source_long", "display_name", "tag"];

/// Timesketch reads tags from the `tag` key: a JSON array in JSONL, a comma-separated list in CSV.
/// Writes and flushes; a flush error (e.g. disk full) is returned, never swallowed.
pub fn write_jsonl(events: &[Event], mut w: impl Write) -> Result<(), CaseError> {
    for e in events {
        let mut obj = serde_json::Map::new();
        obj.insert("datetime".into(), e.datetime.clone().map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
        obj.insert("timestamp_desc".into(), e.timestamp_desc.clone().into());
        obj.insert("message".into(), e.message.clone().into());
        obj.insert("source".into(), e.source.clone().into());
        obj.insert("source_long".into(), e.source_long.clone().into());
        obj.insert("display_name".into(), e.display_name.clone().into());
        obj.insert("tag".into(), serde_json::Value::Array(e.tags.iter().cloned().map(serde_json::Value::String).collect()));
        for (k, v) in &e.attributes {
            if !FIXED.contains(&k.as_str()) && k != "tags" {
                obj.insert(k.clone(), v.clone().into());
            }
        }
        serde_json::to_writer(&mut w, &serde_json::Value::Object(obj))?;
        w.write_all(b"\n")?;
    }
    w.flush()?;
    Ok(())
}

pub fn write_csv(events: &[Event], w: impl Write) -> Result<(), CaseError> {
    let mut extra: BTreeSet<String> = BTreeSet::new();
    for e in events {
        for k in e.attributes.keys() {
            if !FIXED.contains(&k.as_str()) && k != "tags" {
                extra.insert(k.clone());
            }
        }
    }
    let mut wtr = csv::Writer::from_writer(w);
    let mut header: Vec<&str> = FIXED.to_vec();
    header.extend(extra.iter().map(String::as_str));
    wtr.write_record(&header).map_err(|e| CaseError::Export(e.to_string()))?;
    for e in events {
        let mut row: Vec<String> = vec![
            e.datetime.clone().unwrap_or_default(),
            e.timestamp_desc.clone(),
            e.message.clone(),
            e.source.clone(),
            e.source_long.clone(),
            e.display_name.clone(),
            e.tags.join(","),
        ];
        for k in &extra {
            row.push(e.attributes.get(k).cloned().unwrap_or_default());
        }
        wtr.write_record(&row).map_err(|e| CaseError::Export(e.to_string()))?;
    }
    wtr.flush()?;
    Ok(())
}
