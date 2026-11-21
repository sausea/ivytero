# agentero-rss-server

Read-only cloud RSS archive for Agentero. It polls RSS / Atom / JSON Feed
24/7, keeps every article it has seen (no per-feed cap), and serves them over
`GET /api/v1/*` with a read-only bearer token. It stores no reader state:
stars, read marks, tags, AI summaries and Vault paths stay on the client.

## Run

```bash
export RSS_READ_TOKEN=$(openssl rand -hex 32)
cargo run -p agentero-rss-server                 # listens on 127.0.0.1:8787
cargo run -p agentero-rss-server --bin agentero-rss-admin -- feed add https://rss.arxiv.org/rss/cs.AI
```

Docker: `docker compose -f services/agentero-rss-server/compose.yaml up -d`, then
`docker compose ... exec rss agentero-rss-admin feed add <url>`.

In Agentero, open Plaza → Feeds → cloud button, enter the server URL and token,
and switch the source to Cloud.

| Variable | Default |
|---|---|
| `RSS_READ_TOKEN` | required, ≥ 24 chars |
| `RSS_BIND` | `127.0.0.1:8787` |
| `RSS_DB_PATH` | `rss-archive.sqlite` |
| `RSS_DEFAULT_REFRESH_SECS` | `600` (120–86400) |
| `RSS_FETCH_CONCURRENCY` | `4` |
| `RSS_FETCH_TIMEOUT_SECS` | `20` |
| `RSS_MAX_FETCH_BYTES` | `2097152` |
| `RSS_ALLOW_PRIVATE_NETWORKS` | `false` |

## Admin

`agentero-rss-admin [--db PATH] info | feed add|list|disable|enable|set-interval|refresh`.
`disable` stops polling but keeps history and `feedId`; there is no delete.

## API

| Endpoint | Notes |
|---|---|
| `GET /healthz` | no auth |
| `GET /api/v1/info` | `apiVersion`, `serverId`, capabilities |
| `GET /api/v1/feeds[?enabled=true]` | feed health and `articleCount` |
| `GET /api/v1/feeds/{feedId}` | |
| `GET /api/v1/articles?feedId=&limit=&cursor=` | newest first, `limit` 1–200 (default 50), opaque `nextCursor`; no `contentHtml` |
| `GET /api/v1/articles/{articleId}` | full archived HTML |

Errors are `{"error":{"code","message"}}`. Times are UTC RFC 3339 with
milliseconds. Article ids are stable across refetches, edits and restarts.

Backup: stop the service or use `sqlite3 rss-archive.sqlite ".backup out.sqlite"`;
restoring the file keeps `serverId` and every article id.
