mod common;
use common::*;

#[tokio::test]
async fn app_builds_over_an_ingested_case_and_refuses_a_non_case() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    let (status, _) = get_json(&app, "/api/no-such-route").await;
    assert_eq!(status.as_u16(), 404);
    let empty = tempfile::tempdir().unwrap();
    assert!(vem_web::app(empty.path(), PORT).is_err());
}
