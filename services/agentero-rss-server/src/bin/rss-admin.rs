//! Server-side collection config. Not the desktop `agentero feed` command:
//! this edits the cloud archive's `feeds` table directly.

use agentero_core::features::paper::discovery::feeds::parse::normalize_feed_url;
use agentero_rss_server::config::{db_path_from_env, default_refresh_from_env};
use agentero_rss_server::opml;
use agentero_rss_server::storage::{self, Archive};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "agentero-rss-admin",
    about = "Manage Agentero cloud RSS subscriptions"
)]
struct Cli {
    /// SQLite archive path (defaults to $RSS_DB_PATH or ./rss-archive.sqlite).
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print serverId and archive counts.
    Info,
    #[command(subcommand)]
    Feed(FeedCommand),
    #[command(subcommand)]
    Opml(OpmlCommand),
}

#[derive(Subcommand)]
enum OpmlCommand {
    /// Add every feed in an OPML file; existing URLs are skipped.
    Import { file: PathBuf },
    /// Print all feeds as OPML 2.0.
    Export,
}

#[derive(Subcommand)]
enum FeedCommand {
    Add {
        url: String,
        #[arg(long)]
        title: Option<String>,
        /// Seconds between polls (120..=86400).
        #[arg(long)]
        interval: Option<i64>,
    },
    List,
    /// Stop polling; history and feedId are kept.
    Disable {
        feed_id: String,
    },
    Enable {
        feed_id: String,
    },
    SetInterval {
        feed_id: String,
        secs: i64,
    },
    /// Mark due now; the running server polls it on its next tick.
    Refresh {
        feed_id: String,
    },
}

fn run(cli: Cli) -> Result<(), String> {
    let path = cli.db.unwrap_or_else(db_path_from_env);
    let archive = Archive::open(&path).map_err(|e| e.to_string())?;
    let conn = archive.lock();
    let found = |ok: bool, id: &str| {
        if ok {
            Ok(())
        } else {
            Err(format!("feed not found: {id}"))
        }
    };
    match cli.command {
        Command::Info => {
            println!(
                "serverId: {}",
                storage::server_id(&conn).map_err(|e| e.to_string())?
            );
            println!("schema:   {}", storage::SCHEMA_VERSION);
            println!(
                "feeds:    {}",
                storage::list_feeds(&conn, false)
                    .map_err(|e| e.to_string())?
                    .len()
            );
            println!(
                "articles: {}",
                storage::article_count(&conn).map_err(|e| e.to_string())?
            );
        }
        Command::Opml(OpmlCommand::Export) => {
            let feeds = storage::list_feeds(&conn, false).map_err(|e| e.to_string())?;
            print!("{}", opml::export(&feeds, &storage::now_rfc3339()));
        }
        Command::Opml(OpmlCommand::Import { file }) => {
            let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
            let interval = default_refresh_from_env()?;
            let (mut added, mut skipped) = (0, 0);
            for entry in opml::parse(&text)? {
                let Ok(url) = normalize_feed_url(&entry.url) else {
                    eprintln!("skip (invalid url): {}", entry.url);
                    skipped += 1;
                    continue;
                };
                let title = entry.title.unwrap_or_else(|| url.clone());
                match storage::add_feed(&conn, &url, &title, interval) {
                    Ok(feed) => {
                        if !entry.folder.is_empty() {
                            storage::set_folder(&conn, &feed.id, &entry.folder)
                                .map_err(|e| e.to_string())?;
                        }
                        println!("{}\t{}", feed.id, feed.url);
                        added += 1;
                    }
                    Err(e) => {
                        eprintln!("skip ({e})");
                        skipped += 1;
                    }
                }
            }
            eprintln!("added {added}, skipped {skipped}");
        }
        Command::Feed(cmd) => match cmd {
            FeedCommand::Add {
                url,
                title,
                interval,
            } => {
                let url = normalize_feed_url(&url).map_err(|e| e.to_string())?;
                // Untitled feeds keep the url until the first fetch supplies the channel title.
                let title = title.unwrap_or_else(|| url.clone());
                let interval = match interval {
                    Some(i) => i,
                    None => default_refresh_from_env()?,
                };
                let feed =
                    storage::add_feed(&conn, &url, &title, interval).map_err(|e| e.to_string())?;
                println!("{}\t{}", feed.id, feed.url);
            }
            FeedCommand::List => {
                for f in storage::list_feeds(&conn, false).map_err(|e| e.to_string())? {
                    println!(
                        "{}\t{}\t{}s\t{} articles\t{}\t{}{}",
                        f.id,
                        if f.enabled { "enabled" } else { "disabled" },
                        f.refresh_interval_secs,
                        f.article_count,
                        f.last_success_at.as_deref().unwrap_or("never"),
                        f.url,
                        f.last_error
                            .map(|e| format!("\terror: {e}"))
                            .unwrap_or_default(),
                    );
                }
            }
            FeedCommand::Disable { feed_id } => found(
                storage::set_enabled(&conn, &feed_id, false).map_err(|e| e.to_string())?,
                &feed_id,
            )?,
            FeedCommand::Enable { feed_id } => found(
                storage::set_enabled(&conn, &feed_id, true).map_err(|e| e.to_string())?,
                &feed_id,
            )?,
            FeedCommand::SetInterval { feed_id, secs } => found(
                storage::set_interval(&conn, &feed_id, secs).map_err(|e| e.to_string())?,
                &feed_id,
            )?,
            FeedCommand::Refresh { feed_id } => found(
                storage::schedule_now(&conn, &feed_id).map_err(|e| e.to_string())?,
                &feed_id,
            )?,
        },
    }
    Ok(())
}

fn main() {
    if let Err(e) = run(Cli::parse()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
