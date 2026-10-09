//! Vestigo interchange Parquet, format version 1 (verified against Vestigo's `parquet_format.py`).

use super::Event;
use crate::case::now;
use crate::error::CaseError;
use crate::TOOL_VERSION;
use arrow::array::{ArrayRef, ListBuilder, MapBuilder, MapFieldNames, StringBuilder, TimestampMillisecondBuilder, UInt64Builder};
use arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use parquet::file::metadata::KeyValue;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct ParquetReport {
    pub rows: usize,
    pub original_files: usize,
}

fn map_names() -> MapFieldNames {
    MapFieldNames { entry: "entries".to_string(), key: "key".to_string(), value: "value".to_string() }
}

pub fn schema() -> Schema {
    let entries = Field::new(
        "entries",
        DataType::Struct(Fields::from(vec![Field::new("key", DataType::Utf8, false), Field::new("value", DataType::Utf8, true)])),
        false,
    );
    Schema::new(vec![
        Field::new("source_file", DataType::Utf8, false),
        Field::new("file_hash", DataType::Utf8, false),
        Field::new("byte_offset", DataType::UInt64, false),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new("message", DataType::Utf8, false),
        Field::new("timestamp", DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())), true),
        Field::new("timestamp_desc", DataType::Utf8, false),
        Field::new("artifact", DataType::Utf8, false),
        Field::new("artifact_long", DataType::Utf8, false),
        Field::new("display_name", DataType::Utf8, false),
        Field::new("tags", DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))), false),
        Field::new("attributes", DataType::Map(Arc::new(entries), false), false),
    ])
}

fn millis(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|d| d.timestamp_millis())
}

fn arrow_err(e: arrow::error::ArrowError) -> CaseError {
    CaseError::Export(e.to_string())
}

fn parquet_err(e: parquet::errors::ParquetError) -> CaseError {
    CaseError::Export(e.to_string())
}

pub fn write_parquet(events: &[Event], out: &Path) -> Result<ParquetReport, CaseError> {
    let mut source_file = StringBuilder::new();
    let mut file_hash = StringBuilder::new();
    let mut byte_offset = UInt64Builder::new();
    let mut content_hash = StringBuilder::new();
    let mut message = StringBuilder::new();
    let mut timestamp = TimestampMillisecondBuilder::new().with_timezone("UTC");
    let mut timestamp_desc = StringBuilder::new();
    let mut artifact = StringBuilder::new();
    let mut artifact_long = StringBuilder::new();
    let mut display_name = StringBuilder::new();
    let mut tags = ListBuilder::new(StringBuilder::new());
    let mut attributes = MapBuilder::new(Some(map_names()), StringBuilder::new(), StringBuilder::new());
    // Keyed by (sha256, path): identical files at two paths are two original files.
    let mut originals: BTreeMap<(String, String), serde_json::Value> = BTreeMap::new();

    for e in events {
        source_file.append_value(&e.provenance.source_file);
        file_hash.append_value(&e.provenance.file_sha256);
        byte_offset.append_value(e.provenance.byte_offset);
        content_hash.append_value(&e.provenance.content_sha256);
        message.append_value(&e.message);
        match e.datetime.as_deref().and_then(millis) {
            Some(ms) => timestamp.append_value(ms),
            None => timestamp.append_null(),
        }
        timestamp_desc.append_value(&e.timestamp_desc);
        artifact.append_value(&e.source);
        artifact_long.append_value(&e.source_long);
        display_name.append_value(&e.display_name);
        for t in &e.tags {
            tags.values().append_value(t);
        }
        tags.append(true);
        for (k, v) in &e.attributes {
            attributes.keys().append_value(k);
            attributes.values().append_value(v);
        }
        attributes.append(true).map_err(arrow_err)?;
        originals.entry((e.provenance.file_sha256.clone(), e.provenance.source_file.clone())).or_insert_with(|| {
            let name = Path::new(&e.provenance.source_file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| e.provenance.source_file.clone());
            serde_json::json!({ "name": name, "sha256": e.provenance.file_sha256, "size_bytes": e.provenance.file_size, "path": e.provenance.source_file, "mtime": e.provenance.file_mtime })
        });
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(source_file.finish()),
        Arc::new(file_hash.finish()),
        Arc::new(byte_offset.finish()),
        Arc::new(content_hash.finish()),
        Arc::new(message.finish()),
        Arc::new(timestamp.finish()),
        Arc::new(timestamp_desc.finish()),
        Arc::new(artifact.finish()),
        Arc::new(artifact_long.finish()),
        Arc::new(display_name.finish()),
        Arc::new(tags.finish()),
        Arc::new(attributes.finish()),
    ];
    let original_files: Vec<serde_json::Value> = originals.into_values().collect();
    let footer: Vec<(String, String)> = vec![
        ("vestigo.format_version".to_string(), "1".to_string()),
        ("vestigo.converter_name".to_string(), "vem".to_string()),
        ("vestigo.converter_version".to_string(), TOOL_VERSION.to_string()),
        ("vestigo.original_files".to_string(), serde_json::to_string(&original_files)?),
        ("vestigo.converted_at".to_string(), now()),
        ("vestigo.row_counts".to_string(), serde_json::json!({ "parsed": events.len(), "skipped_malformed": 0, "skipped_by_time": 0 }).to_string()),
        ("vestigo.timezone_assumption".to_string(), "all timestamps stored UTC by vem; origin per row in timestamp_desc and attributes.ts_origin".to_string()),
    ];
    // Vestigo reads `schema_arrow.metadata`, which pyarrow rebuilds from the serialized Arrow schema
    // (`ARROW:schema`), so the keys must live on the Arrow schema. They are also kept as plain
    // Parquet footer key/value entries for readers that look there.
    let schema = Arc::new(schema().with_metadata(footer.iter().cloned().collect::<HashMap<_, _>>()));
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(arrow_err)?;
    let metadata: Vec<KeyValue> = footer.into_iter().map(|(k, v)| KeyValue::new(k, v)).collect();
    let props = WriterProperties::builder().set_key_value_metadata(Some(metadata)).build();
    let file = std::fs::File::create(out)?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).map_err(parquet_err)?;
    writer.write(&batch).map_err(parquet_err)?;
    writer.close().map_err(parquet_err)?;
    Ok(ParquetReport { rows: events.len(), original_files: original_files.len() })
}
