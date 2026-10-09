mod common;

use common::*;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::parquet::write_parquet;
use vem_case::export::{events, Scope};
use vem_case::ingest::ingest;
use vem_case::Case;

#[test]
fn writes_vestigo_v1_schema_and_footer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), AttachOptions { label: "alice".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    let ev = events(&case, &Scope::Case).unwrap();
    let out = tmp.path().join("case.parquet");
    let report = write_parquet(&case, &ev, &out).unwrap();
    assert_eq!(report.rows, ev.len());
    assert!(report.original_files >= 4, "S1, S0, subagent, orphan transcripts and history.jsonl");

    let builder = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&out).unwrap()).unwrap();
    let schema = builder.schema().clone();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(names, vec!["source_file", "file_hash", "byte_offset", "content_hash", "message", "timestamp", "timestamp_desc", "artifact", "artifact_long", "display_name", "tags", "attributes"]);
    use arrow::datatypes::{DataType, TimeUnit};
    assert_eq!(schema.field_with_name("byte_offset").unwrap().data_type(), &DataType::UInt64);
    assert_eq!(schema.field_with_name("timestamp").unwrap().data_type(), &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())));
    assert!(matches!(schema.field_with_name("tags").unwrap().data_type(), DataType::List(_)));
    assert!(matches!(schema.field_with_name("attributes").unwrap().data_type(), DataType::Map(_, _)));
    // Vestigo validates pyarrow's `ParquetFile.schema_arrow.metadata`, which pyarrow rebuilds from the
    // embedded `ARROW:schema` entry alone. arrow-rs's reader merges the footer key/values into the schema
    // metadata, so decode the embedded schema with only that entry, the way pyarrow sees it.
    let fm = builder.metadata().file_metadata();
    let arrow_only: Vec<parquet::file::metadata::KeyValue> =
        fm.key_value_metadata().into_iter().flatten().filter(|kv| kv.key == parquet::arrow::ARROW_SCHEMA_META_KEY).cloned().collect();
    assert_eq!(arrow_only.len(), 1, "file must embed an Arrow schema");
    let embedded = parquet::arrow::parquet_to_arrow_schema(fm.schema_descr(), Some(&arrow_only)).unwrap();
    let arrow_meta = embedded.metadata();
    assert_eq!(arrow_meta.get("vestigo.format_version").map(String::as_str), Some("1"), "Arrow schema metadata must carry the Vestigo keys");
    for k in ["vestigo.converter_name", "vestigo.converter_version", "vestigo.original_files"] {
        assert!(arrow_meta.contains_key(k), "Arrow schema metadata missing {k}");
    }
    let kv = builder.metadata().file_metadata().key_value_metadata().cloned().unwrap_or_default();
    let get = |k: &str| kv.iter().find(|x| x.key == k).and_then(|x| x.value.clone()).unwrap_or_else(|| panic!("missing footer key {k}"));
    assert_eq!(get("vestigo.format_version"), "1");
    assert_eq!(get("vestigo.converter_name"), "vem");
    assert_eq!(get("vestigo.converter_version"), vem_case::TOOL_VERSION);
    let originals: Vec<serde_json::Value> = serde_json::from_str(&get("vestigo.original_files")).unwrap();
    assert_eq!(originals.len(), report.original_files);
    assert!(originals.iter().all(|o| o["name"].is_string() && o["sha256"].as_str().unwrap().len() == 64 && o["size_bytes"].is_number()));
    let total: usize = builder.build().unwrap().map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(total, ev.len());
}
