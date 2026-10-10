mod common;

use common::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::Case;

fn exposure() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/exposure")
        .canonicalize()
        .unwrap()
}

fn ingested(root: &Path) -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(
        &mut case,
        root,
        AttachOptions {
            label: "x".into(),
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

/// rule -> (field, linked to block, linked to tool call, linked to provenance)
fn secrets(case: &Case) -> BTreeMap<String, (String, bool, bool, bool)> {
    let mut stmt = case
        .conn
        .prepare("SELECT json_extract(details, '$.rule'), json_extract(details, '$.field'), derived_from_block_id IS NOT NULL, derived_from_tool_call_id IS NOT NULL, derived_from_provenance_id IS NOT NULL FROM observations WHERE kind = 'secret_candidate'")
        .unwrap();
    stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?),
        ))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

#[test]
fn exposure_secrets_are_derived_and_linked() {
    let (_t, case) = ingested(&exposure());
    let s = secrets(&case);
    assert_eq!(
        s.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "anthropic-key",
            "aws-access-key-id",
            "generic-assignment",
            "github-token",
            "private-key"
        ]
    );
    assert_eq!(s["private-key"], ("text".to_string(), true, false, false));
    assert_eq!(
        s["github-token"],
        ("tool_input".to_string(), false, true, false)
    );
    assert_eq!(
        s["generic-assignment"],
        ("tool_input".to_string(), false, true, false)
    );
    assert_eq!(
        s["aws-access-key-id"],
        ("paste".to_string(), false, false, true)
    );
    assert_eq!(
        s["anthropic-key"],
        ("paste".to_string(), false, false, true)
    );
    let (confidence, matched): (String, String) = case
        .conn
        .query_row("SELECT confidence, json_extract(details, '$.match') FROM observations WHERE kind = 'secret_candidate' AND json_extract(details, '$.rule') = 'github-token'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(confidence, "high");
    assert!(matched.starts_with("ghp_"));
    let rules: String = case
        .conn
        .query_row(
            "SELECT json_extract(details, '$.secret_rules') FROM audit_log WHERE action = 'ingest'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rules, vem_case::secrets::RULESET_VERSION);
}

#[test]
fn basic_gains_exactly_the_pasted_aws_key_and_reingest_adds_nothing() {
    let (_t, mut case) = ingested(&fixture_root());
    let s = secrets(&case);
    assert_eq!(s.len(), 1);
    assert_eq!(s["aws-access-key-id"].0, "paste");
    ingest(&mut case, None).unwrap();
    assert_eq!(
        case.conn
            .query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM observations WHERE kind = 'secret_candidate'",
                [],
                |r| r.get(0)
            )
            .unwrap(),
        1
    );
}
