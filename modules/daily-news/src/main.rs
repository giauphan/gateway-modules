//! Standalone Native Rust stdio-stream entrypoint for Daily News.
//!
//! Frameworks & Drivers layer: Composition root and stdio-stream event loop.
//! Reads JSON requests from stdin, dispatches to interface adapter handlers,
//! and writes JSON responses to stdout.

use daily_news::discussion::LiveDiscussionFetcher;
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::handlers::handle_request;
use daily_news::reddit::RedditFeedProvider;
use daily_news::redis::RedisFeedProvider;
use daily_news::rpc::make_json_resp;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::{FileStorageAdapter, SqliteStorageAdapter};
use daily_news::StoragePort;
use serde_json::{json, Value};
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let base_dir = std::env::var("DAILY_NEWS_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if cur.join("modules").join("daily-news").is_dir() {
                cur.join("modules").join("daily-news")
            } else {
                cur
            }
        });
    let use_sqlite = match std::env::var("DAILY_NEWS_STORAGE_ENGINE") {
        Ok(engine) => engine.eq_ignore_ascii_case("sqlite"),
        Err(_) => {
            std::env::var("DAILY_NEWS_ENV").as_deref() == Ok("production")
                || std::env::var("GATEWAY_ENV").as_deref() == Ok("production")
        }
    };

    let storage: Arc<dyn StoragePort> = if use_sqlite {
        match SqliteStorageAdapter::new(&base_dir) {
            Ok(sqlite) => Arc::new(sqlite),
            Err(e) => {
                eprintln!("[daily-news] FATAL: SQLite storage engine configured, but initialization failed: {e}");
                eprintln!("[daily-news] Failing closed to prevent split-brain state or duplicate publish claims.");
                std::process::exit(1);
            }
        }
    } else {
        Arc::new(FileStorageAdapter::new(&base_dir))
    };
    let initial_config = storage.load_config();
    let redis_provider = initial_config.redis_enabled.then(|| {
        RedisFeedProvider::new(
            &initial_config.redis_url,
            &initial_config.redis_key,
            &initial_config.redis_type,
        )
    });

    let reddit_provider = initial_config.reddit_enabled.then(|| {
        RedditFeedProvider::new(initial_config.reddit_subreddits.clone())
            .with_credentials(
                initial_config.reddit_client_id.clone(),
                initial_config.reddit_client_secret.clone(),
            )
            .with_base_url(initial_config.reddit_base_url.clone())
    });

    let mut fetcher = LiveFeedFetcher::new(Some(initial_config.newsnow_url));
    if let Some(rp) = redis_provider {
        fetcher = fetcher.with_redis(rp);
    }
    if let Some(rp) = reddit_provider {
        fetcher = fetcher.with_reddit(rp);
    }
    let fetcher = Arc::new(fetcher);
    let scorer = CandidateScorer::default();
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&base_dir));
    let service = DailyNewsService::new(fetcher, storage, scorer)
        .with_discussion_fetcher(Arc::new(LiveDiscussionFetcher::new()))
        .with_source_capture(capturer);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err_resp =
                    make_json_resp(400, json!({ "error": format!("Invalid JSON: {e}") }));
                println!("{}", serde_json::to_string(&err_resp).unwrap_or_default());
                continue;
            }
        };

        let resp = handle_request(req, &service, &base_dir);
        println!("{}", serde_json::to_string(&resp).unwrap_or_default());
    }
}
