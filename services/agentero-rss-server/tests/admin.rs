//! Admin UI/API: off without RSS_ADMIN_TOKEN, separate token, OPML round trip.

use agentero_rss_server::api::{router, AppState};
use agentero_rss_server::storage::Archive;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

const READ: &str = "test-read-token-0123456789abcdef";
const ADMIN: &str = "test-admin-token-0123456789abcdef";

fn app(dir: &tempfile::TempDir, admin: bool) -> axum::Router {
    let archive = Archive::open(&dir.path().join("a.sqlite")).unwrap();
    router(AppState {
        archive,
        read_token: Arc::new(READ.into()),
        admin_token: admin.then(|| Arc::new(ADMIN.to_string())),
        default_refresh_secs: 600,
    })
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    if body.starts_with('{') {
        req = req.header("content-type", "application/json");
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn admin_is_off_without_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, false);
    assert_eq!(
        call(&app, "GET", "/admin", None, "").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "GET", "/api/admin/feeds", Some(READ), "")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn opml_import_export_and_feed_edits() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, true);
    let (status, html) = call(&app, "GET", "/admin", None, "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("/admin/app.js"));
    // The read token cannot manage feeds, and the read API stays GET-only.
    assert_eq!(
        call(&app, "GET", "/api/admin/feeds", Some(READ), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "GET", "/api/v1/feeds", Some(ADMIN), "").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "POST", "/api/v1/feeds", Some(READ), "{}")
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );

    let opml = r#"<?xml version="1.0"?><opml version="2.0"><body>
        <outline text="Folder"><outline text="A &amp; B" xmlUrl="https://a.example/feed"/></outline>
        <outline text="Dup" xmlUrl="https://a.example/feed"/>
        <outline text="Bad" xmlUrl="ftp://x.example/feed"/>
        <outline title="中文" xmlUrl="https://b.example/rss"/></body></opml>"#;
    let (status, body) = call(&app, "POST", "/api/admin/opml", Some(ADMIN), opml).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(report["added"].as_array().unwrap().len(), 2);
    let reasons: Vec<_> = report["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["duplicate", "invalid_url"]);
    assert_eq!(
        call(&app, "POST", "/api/admin/opml", Some(ADMIN), "<rss/>")
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    let (status, _) = call(
        &app,
        "POST",
        "/api/admin/feeds",
        Some(ADMIN),
        r#"{"url":"https://b.example/rss"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, body) = call(
        &app,
        "POST",
        "/api/admin/feeds",
        Some(ADMIN),
        r#"{"url":"https://c.example/atom","title":"C"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, body) = call(
        &app,
        "PATCH",
        &format!("/api/admin/feeds/{id}"),
        Some(ADMIN),
        r#"{"enabled":false,"intervalSecs":3600,"title":"C2"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let feed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (
            feed["enabled"].as_bool(),
            feed["refreshIntervalSecs"].as_i64(),
            feed["title"].as_str()
        ),
        (Some(false), Some(3600), Some("C2"))
    );
    assert_eq!(
        call(
            &app,
            "PATCH",
            &format!("/api/admin/feeds/{id}"),
            Some(ADMIN),
            r#"{"intervalSecs":5}"#
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/admin/feeds/{id}/refresh"),
            Some(ADMIN),
            ""
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/admin/feeds/missing/refresh",
            Some(ADMIN),
            ""
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let (status, xml) = call(&app, "GET", "/api/admin/opml", Some(ADMIN), "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        xml.contains(r#"text="A &amp; B""#)
            && xml.contains("https://b.example/rss")
            && xml.contains(r#"title="C2""#)
    );
    let reparsed = agentero_rss_server::opml::parse(&xml).unwrap();
    assert_eq!(reparsed.len(), 3);
}
