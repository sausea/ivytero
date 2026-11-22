//! Title translation: stored per source title, skipped for Chinese, retried on failure.

use agentero_rss_server::ingest::translator::translate_batch;
use agentero_rss_server::storage::{self, Archive, IncomingItem};

fn item(guid: &str, title: &str) -> IncomingItem {
    IncomingItem {
        guid: guid.into(),
        title: title.into(),
        url: None,
        published_at: None,
        summary_text: String::new(),
        content_html: None,
        paper_url: None,
    }
}

#[tokio::test]
async fn translates_titles_and_redoes_changed_ones() {
    let dir = tempfile::tempdir().unwrap();
    let archive = Archive::open(&dir.path().join("a.sqlite")).unwrap();
    let feed = archive
        .run(|c| storage::add_feed(c, "https://a.example/feed", "A", 600))
        .await
        .unwrap();
    let id = feed.id.clone();
    let items = vec![
        item("1", "Fed holds rates"),
        item("2", "美联储维持利率不变"),
        item("3", "fails"),
    ];
    archive
        .run(move |c| {
            storage::record_success(c, &id, None, &items, None, None, "2030-01-01T00:00:00.000Z")
        })
        .await
        .unwrap();

    let fake = |t: String| async move { (t != "fails").then(|| format!("译:{t}")) };
    assert_eq!(translate_batch(&archive, fake).await.unwrap(), (2, 1));
    let titles = |archive: Archive| async move {
        let (rows, _) = archive
            .run(|c| storage::list_articles(c, None, None, None, 10, false))
            .await
            .unwrap();
        let mut out: Vec<(String, Option<String>)> = rows
            .into_iter()
            .map(|a| (a.title, a.title_translated))
            .collect();
        out.sort();
        out
    };
    assert_eq!(
        titles(archive.clone()).await,
        [
            (
                "Fed holds rates".to_string(),
                Some("译:Fed holds rates".to_string())
            ),
            ("fails".to_string(), None),
            ("美联储维持利率不变".to_string(), None),
        ]
    );
    // Only the failed one is still pending.
    let pending = archive
        .run(|c| storage::pending_title_translations(c, 10))
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);

    // A title edit hides the stale translation until it is redone.
    let id = feed.id.clone();
    let items = vec![item("1", "Fed cuts rates")];
    archive
        .run(move |c| {
            storage::record_success(c, &id, None, &items, None, None, "2030-01-01T00:00:00.000Z")
        })
        .await
        .unwrap();
    assert!(titles(archive.clone())
        .await
        .iter()
        .all(|(t, tr)| t != "Fed cuts rates" || tr.is_none()));
    let ok = |t: String| async move { Some(format!("译:{t}")) };
    translate_batch(&archive, ok).await.unwrap();
    assert!(titles(archive.clone()).await.contains(&(
        "Fed cuts rates".to_string(),
        Some("译:Fed cuts rates".to_string())
    )));
}
