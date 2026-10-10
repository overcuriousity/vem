mod common;

use common::*;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::{self, Format, Scope};
use vem_case::{Case, CaseError};

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(
        &mut case,
        &fixture_root(),
        AttachOptions {
            label: "alice".into(),
            host: None,
            user: None,
            os: None,
            harness: None,
            retain: true,
        },
    )
    .unwrap();
    vem_case::ingest::ingest(&mut case, None).unwrap();
    (tmp, case)
}

#[test]
fn run_writes_hashes_audits_and_lists() {
    let (_t, case) = ingested();
    let out = export::default_output(&case, Format::TimesketchJsonl, &Scope::Case);
    assert!(out.starts_with(case.dir.join("exports")));
    assert!(out
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("-case.jsonl"));
    let r = export::run(&case, Format::TimesketchJsonl, &Scope::Case, &out).unwrap();
    assert_eq!(r.events, export::events(&case, &Scope::Case).unwrap().len());
    assert_eq!(r.sha256, vem_core::hash::sha256_file(&out).unwrap().0);
    assert_eq!(r.name, out.file_name().unwrap().to_str().unwrap());
    let second = export::default_output(&case, Format::TimesketchJsonl, &Scope::Case);
    assert_ne!(
        second, out,
        "a second export in the same second gets its own name"
    );
    let listed = export::list(&case).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].name.as_str(),
            listed[0].exists,
            listed[0].format.as_str()
        ),
        (r.name.as_str(), true, "timesketch-jsonl")
    );
    assert_eq!(export::export_file(&case, &r.name).unwrap(), out);
    for bad in ["../case.db", "..", ".", "a/b", "nope.jsonl", ""] {
        assert!(
            matches!(export::export_file(&case, bad), Err(CaseError::NotFound(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn run_refuses_unknown_scopes_and_formats_parse() {
    let (_t, case) = ingested();
    let out = case.dir.join("exports/x.csv");
    assert!(matches!(
        export::run(&case, Format::TimesketchCsv, &Scope::Session(999_999), &out),
        Err(CaseError::NoSuchSession(_))
    ));
    assert_eq!(
        Format::parse("vestigo-parquet"),
        Some(Format::VestigoParquet)
    );
    assert_eq!(Format::VestigoParquet.extension(), "parquet");
    assert_eq!(Format::parse("xml"), None);
    assert_eq!(export::scope_label(&Scope::Root(3)), "root-3");
}
