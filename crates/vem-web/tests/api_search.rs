mod common;
use common::*;

#[tokio::test]
async fn search_enriches_hits_and_never_errors_on_syntax() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let (s, hits) = get_json(&app, "/api/search?q=notes%20file").await;
    assert_eq!(s, 200);
    let h = &hits.as_array().unwrap()[0];
    assert!(
        h["session_title"].is_string()
            && h["message_ordinal"].is_i64()
            && h["snippet"].as_str().unwrap().contains('[')
    );
    let (_, gh) = get_json(&app, "/api/search?q=gh%20auth%20status").await;
    assert!(
        gh.as_array()
            .unwrap()
            .iter()
            .any(|h| h["tool_call_id"].is_i64()),
        "tool inputs are searchable"
    );
    for q in ["%22", "*", "(", "NEAR", "-x", "%22*(%20NEAR%20-"] {
        let (s, _) = get_json(&app, &format!("/api/search?q={q}")).await;
        assert_eq!(s, 200, "{q}");
    }
    let (_, blank) = get_json(&app, "/api/search?q=%20%20").await;
    assert!(blank.as_array().unwrap().is_empty());
    let (_, missing) = get_json(&app, "/api/search").await;
    assert!(missing.as_array().unwrap().is_empty());
    let (_, one) = get_json(&app, "/api/search?q=notes&limit=1").await;
    assert!(one.as_array().unwrap().len() <= 1);
}
