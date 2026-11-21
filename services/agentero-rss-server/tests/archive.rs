//! End-to-end: feed origin → poll → archive → read-only API.

use agentero_rss_server::api::{router, AppState};
use agentero_rss_server::ingest::fetcher::FetchPolicy;
use agentero_rss_server::ingest::scheduler::{poll_feed, PollResult};
use agentero_rss_server::storage::{self, Archive};
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const TOKEN: &str = "test-read-token-0123456789abcdef";

fn rss(items: &[(&str, &str, &str)]) -> String {
    let body: String = items
        .iter()
        .map(|(guid, title, date)| {
            format!("<item><guid>{guid}</guid><title>{title}</title><link>https://example.org/{guid}</link><pubDate>{date}</pubDate><description>about {title}</description></item>")
        })
        .collect();
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Origin Feed</title>{body}</channel></rss>"#
    )
}

#[derive(Clone, Default)]
struct Origin {
    body: Arc<Mutex<String>>,
    hits: Arc<Mutex<u32>>,
}

async fn serve_origin(origin: Origin) -> String {
    let app = axum::Router::new().route(
        "/feed.xml",
        get(move |headers: HeaderMap| {
            let origin = origin.clone();
            async move {
                *origin.hits.lock().unwrap() += 1;
                let body = origin.body.lock().unwrap().clone();
                let etag = format!("\"{}\"", body.len());
                if headers
                    .get(header::IF_NONE_MATCH)
                    .and_then(|v| v.to_str().ok())
                    == Some(etag.as_str())
                {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
                (
                    [
                        (header::CONTENT_TYPE, "application/rss+xml".to_string()),
                        (header::ETAG, etag),
                    ],
                    body,
                )
                    .into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/feed.xml")
}

fn policy() -> FetchPolicy {
    FetchPolicy {
        allow_private_networks: true,
        ..FetchPolicy::default()
    }
}

async fn poll(archive: &Archive, feed_id: &str) -> PollResult {
    let id = feed_id.to_string();
    let due = archive
        .run(move |c| {
            Ok(storage::due_feeds(c, "9999-01-01T00:00:00.000Z", 100)?
                .into_iter()
                .find(|f| f.id == id)
                .unwrap())
        })
        .await
        .unwrap();
    poll_feed(archive, &policy(), due).await.unwrap()
}

async fn get_json(app: &axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut req = Request::get(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn archives_full_history_with_stable_ids_and_pages() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("archive.sqlite");
    let origin = Origin::default();
    *origin.body.lock().unwrap() = rss(&[
        ("a", "Alpha", "Mon, 02 Nov 2026 10:00:00 GMT"),
        ("b", "Beta", "Tue, 03 Nov 2026 10:00:00 GMT"),
        ("c", "Gamma", "Tue, 03 Nov 2026 10:00:00 GMT"),
    ]);
    let url = serve_origin(origin.clone()).await;

    let archive = Archive::open(&db).unwrap();
    let feed = archive
        .run(move |c| storage::add_feed(c, &url, "", 600))
        .await
        .unwrap();
    assert!(matches!(poll(&archive, &feed.id).await, PollResult::Archived(s) if s.inserted == 3));
    assert_eq!(poll(&archive, &feed.id).await, PollResult::NotModified);

    // Origin drops "a" and edits "b": history keeps "a", "b" keeps its id.
    let first_ids: Vec<(String, String)> = archive
        .run(|c| {
            Ok(storage::list_articles(c, None, None, 10, false)?
                .0
                .into_iter()
                .map(|a| (a.guid, a.id))
                .collect())
        })
        .await
        .unwrap();
    *origin.body.lock().unwrap() = rss(&[
        ("b", "Beta v2", "Tue, 03 Nov 2026 10:00:00 GMT"),
        ("d", "Delta", "Wed, 04 Nov 2026 10:00:00 GMT"),
    ]);
    let stats = match poll(&archive, &feed.id).await {
        PollResult::Archived(s) => s,
        other => panic!("{other:?}"),
    };
    assert_eq!((stats.inserted, stats.updated, stats.unchanged), (1, 1, 0));
    drop(archive);

    // Reopen: same serverId, same article ids, nothing pruned.
    let archive = Archive::open(&db).unwrap();
    let app = router(AppState {
        archive: archive.clone(),
        read_token: Arc::new(TOKEN.into()),
    });
    let (status, info) = get_json(&app, "/api/v1/info", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(info["capabilities"]["userState"], false);
    let server_id = archive.run(|c| storage::server_id(c)).await.unwrap();
    assert_eq!(info["serverId"], server_id);

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = match &cursor {
            Some(c) => format!("/api/v1/articles?limit=1&feedId={}&cursor={c}", feed.id),
            None => format!("/api/v1/articles?limit=1&feedId={}", feed.id),
        };
        let (status, page) = get_json(&app, &uri, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        for item in page["items"].as_array().unwrap() {
            assert!(item["contentHtml"].is_null());
            seen.push((
                item["guid"].as_str().unwrap().to_string(),
                item["id"].as_str().unwrap().to_string(),
            ));
        }
        match page["nextCursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    let guids: Vec<&str> = seen.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(guids.len(), 4);
    assert_eq!(guids[0], "d");
    assert_eq!(guids[3], "a");
    for (guid, id) in &first_ids {
        assert!(
            seen.contains(&(guid.clone(), id.clone())),
            "id for {guid} changed"
        );
    }

    let b_id = &seen.iter().find(|(g, _)| g == "b").unwrap().1;
    let (status, article) = get_json(&app, &format!("/api/v1/articles/{b_id}"), Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(article["title"], "Beta v2");
    assert_eq!(article["feedTitle"], "Origin Feed");

    let (status, feeds) = get_json(&app, "/api/v1/feeds", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(feeds["items"][0]["articleCount"], 4);
    assert!(feeds["items"][0].get("etag").is_none());
}

#[tokio::test]
async fn api_rejects_bad_auth_input_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let archive = Archive::open(&dir.path().join("a.sqlite")).unwrap();
    let app = router(AppState {
        archive,
        read_token: Arc::new(TOKEN.into()),
    });

    let (status, body) = get_json(&app, "/api/v1/feeds", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    let (status, _) = get_json(&app, "/api/v1/feeds", Some("wrong-token-wrong-token-xx")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    for uri in [
        "/api/v1/articles?limit=0",
        "/api/v1/articles?limit=201",
        "/api/v1/articles?cursor=%%%",
        "/api/v1/articles?cursor=Zm9v",
    ] {
        let (status, body) = get_json(&app, uri, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["error"]["code"], "bad_request");
    }
    let (status, _) = get_json(&app, "/api/v1/articles/nope", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let req = Request::put("/api/v1/articles/x/star")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(res.status().is_client_error());

    let res = app
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_keeps_history_and_backs_off() {
    let dir = tempfile::tempdir().unwrap();
    let archive = Archive::open(&dir.path().join("a.sqlite")).unwrap();
    let origin = Origin::default();
    *origin.body.lock().unwrap() = rss(&[("a", "Alpha", "Mon, 02 Nov 2026 10:00:00 GMT")]);
    let url = serve_origin(origin.clone()).await;
    let feed = archive
        .run(move |c| storage::add_feed(c, &url, "", 600))
        .await
        .unwrap();
    poll(&archive, &feed.id).await;

    *origin.body.lock().unwrap() = "<html>not a feed</html>".into();
    assert!(matches!(
        poll(&archive, &feed.id).await,
        PollResult::Failed(_)
    ));
    let id = feed.id.clone();
    let row = archive
        .run(move |c| Ok(storage::get_feed(c, &id)?.unwrap()))
        .await
        .unwrap();
    assert_eq!(row.failure_count, 1);
    assert_eq!(row.article_count, 1);
    assert!(row.last_error.is_some());
    assert!(row.next_fetch_at.unwrap() > storage::now_rfc3339());
}
