mod common;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::*;

fn with_host(method: Method, path: &str, host: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", host)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn only_loopback_host_headers_are_answered() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    for bad in [
        "evil.example:8787",
        "127.0.0.1:9999",
        "localhost",
        "",
        "127.0.0.1.evil.example:8787",
    ] {
        let (s, _, b) = send(&app, with_host(Method::GET, "/api/case", bad)).await;
        assert_eq!(s, StatusCode::MISDIRECTED_REQUEST, "{bad:?}");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&b).unwrap()["kind"],
            "bad_host"
        );
    }
    for good in ["127.0.0.1:8787", "localhost:8787"] {
        let (s, _, _) = send(&app, with_host(Method::GET, "/", good)).await;
        assert_eq!(s, StatusCode::OK, "{good}");
    }
}

#[tokio::test]
async fn writes_need_json_and_a_local_origin() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    let post = |ct: Option<&str>, origin: Option<&str>| {
        let mut b = req(Method::POST, "/api/annotations");
        if let Some(c) = ct {
            b = b.header("content-type", c);
        }
        if let Some(o) = origin {
            b = b.header("origin", o);
        }
        b.body(Body::from("{}")).unwrap()
    };
    assert_eq!(
        send(&app, post(None, Some(ORIGIN))).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, post(Some("text/plain"), Some(ORIGIN))).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            post(Some("application/json"), Some("http://evil.example"))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            post(Some("application/json"), Some("http://127.0.0.1:9999"))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_ne!(
        send(&app, post(Some("application/json"), Some(ORIGIN)))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_ne!(
        send(&app, post(Some("application/json; charset=utf-8"), None))
            .await
            .0,
        StatusCode::FORBIDDEN,
        "no Origin: a local tool such as curl"
    );
}

#[tokio::test]
async fn security_headers_on_every_response() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    for path in ["/", "/api/no-such-route", "/sessions/1"] {
        let (_, h, _) = send(&app, req(Method::GET, path).body(Body::empty()).unwrap()).await;
        let csp = h.get("content-security-policy").unwrap().to_str().unwrap();
        assert!(csp.contains("default-src 'self'"), "{path}: {csp}");
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(h.get("referrer-policy").unwrap(), "no-referrer");
    }
    let (_, h, _) = send(
        &app,
        req(Method::GET, "/api/no-such-route")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(h.get("cache-control").unwrap(), "no-store");
    let (_, h, _) = send(&app, with_host(Method::GET, "/", "evil.example")).await;
    assert!(
        h.get("content-security-policy").is_some(),
        "rejections carry the headers too"
    );
}
