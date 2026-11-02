//! Network feed fetcher implementing `FeedFetcherPort`.
//!
//! Pulls breaking stories from Hacker News, NewsNow, and Redis providers using `ureq`.

use crate::domain::{extract_domain, format_utc_timestamp, RawNewsItem};
use crate::ports::FeedFetcherPort;
use crate::redis::RedisFeedProvider;
use serde_json::Value;
use std::collections::HashMap;

pub struct LiveFeedFetcher {
    pub newsnow_url: Option<String>,
    pub redis_provider: Option<RedisFeedProvider>,
}

impl LiveFeedFetcher {
    pub fn new(newsnow_url: Option<String>) -> Self {
        Self {
            newsnow_url,
            redis_provider: None,
        }
    }

    pub fn with_redis(mut self, redis: RedisFeedProvider) -> Self {
        self.redis_provider = Some(redis);
        self
    }

    fn fetch_newsnow_source(&self, base_url: &str, source: &str, limit: usize) -> Vec<RawNewsItem> {
        let endpoint = format!("{}/api/s?id={source}", base_url.trim_end_matches('/'));
        let val: Value = match ureq::get(&endpoint)
            .header("Accept", "application/json")
            .header("User-Agent", "toolcase-daily-news/1.3.0")
            .call()
        {
            Ok(res) => res.into_body().read_json().unwrap_or_default(),
            Err(_) => return Vec::new(),
        };

        let items_arr = match val["items"].as_array() {
            Some(arr) => arr,
            None => return Vec::new(),
        };

        let mut items = Vec::new();
        for raw in items_arr.iter().take(limit) {
            let title = raw["title"].as_str().unwrap_or("").trim().to_string();
            let url = raw["url"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() || url.is_empty() {
                continue;
            }
            let item_id = raw["id"].as_str().unwrap_or(&url);
            let publisher = extract_domain(&url);
            let pub_timestamp = raw["published_at"]
                .as_str()
                .or_else(|| raw["pubDate"].as_str())
                .unwrap_or("2026-11-02T11:00:00+00:00")
                .to_string();

            let disc_url = raw["discussion_url"]
                .as_str()
                .or_else(|| raw["comments_url"].as_str())
                .map(str::to_string);

            items.push(RawNewsItem {
                id: format!("newsnow_{source}_{item_id}"),
                url,
                title,
                published_at: pub_timestamp,
                publisher: Some(publisher),
                author: raw["author"].as_str().map(str::to_string),
                description: raw["description"].as_str().map(str::to_string),
                body: raw["body"].as_str().map(str::to_string),
                discussion_url: disc_url,
                tags: vec![source.to_string()],
                is_evergreen: false,
                content_kind: Some("news".to_string()),
                updated_at: None,
                extra: HashMap::new(),
            });
        }
        items
    }

    fn fetch_hackernews(&self, limit: usize) -> Vec<RawNewsItem> {
        let endpoint = "https://hacker-news.firebaseio.com/v0/topstories.json";
        let ids: Vec<u64> = match ureq::get(endpoint).call() {
            Ok(res) => res.into_body().read_json().unwrap_or_default(),
            Err(_) => Vec::new(),
        };

        let target_ids: Vec<u64> = ids.into_iter().take(limit.max(25)).collect();
        let mut items = Vec::new();

        for id in target_ids {
            let item_url = format!("https://hacker-news.firebaseio.com/v0/item/{id}.json");
            let val: Value = match ureq::get(&item_url).call() {
                Ok(res) => res.into_body().read_json().unwrap_or_default(),
                Err(_) => continue,
            };

            let title = val["title"].as_str().unwrap_or("").trim().to_string();
            let is_external = val["url"].is_string();
            let orig_url = val["url"]
                .as_str()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={id}"));
            if title.is_empty() {
                continue;
            }

            let publisher = if is_external {
                extract_domain(&orig_url)
            } else {
                "news.ycombinator.com".to_string()
            };

            let disc_url = format!("https://news.ycombinator.com/item?id={id}");
            let text = val["text"].as_str().map(str::to_string);
            let author = val["by"].as_str().map(str::to_string);

            let pub_timestamp = val["time"]
                .as_u64()
                .map(format_utc_timestamp)
                .unwrap_or_else(|| "2026-11-02T11:00:00+00:00".to_string());

            let mut extra = HashMap::new();
            if let Some(ref by) = author {
                extra.insert("by".to_string(), Value::String(by.clone()));
            }
            if let Some(score) = val["score"].as_i64() {
                extra.insert("score".to_string(), Value::Number(score.into()));
            }
            if let Some(comments) = val["descendants"].as_i64() {
                extra.insert("comments".to_string(), Value::Number(comments.into()));
            }
            extra.insert(
                "discussion_url".to_string(),
                Value::String(disc_url.clone()),
            );

            items.push(RawNewsItem {
                id: format!("hn_{id}"),
                url: orig_url,
                title,
                published_at: pub_timestamp,
                publisher: Some(publisher),
                author,
                description: text.clone(),
                body: text,
                discussion_url: Some(disc_url),
                tags: vec!["hackernews".to_string(), "tech".to_string()],
                is_evergreen: false,
                content_kind: Some("news".to_string()),
                updated_at: None,
                extra,
            });
        }
        items
    }
}

impl FeedFetcherPort for LiveFeedFetcher {
    fn fetch_raw_items(&self, limit: usize) -> Result<Vec<RawNewsItem>, String> {
        let mut items = Vec::new();
        if let Some(ref redis) = self.redis_provider {
            items.extend(redis.fetch_items(limit));
        }

        if let Some(ref nn_url) = self.newsnow_url {
            let nn_items = self.fetch_newsnow_source(nn_url, "hackernews", limit);
            items.extend(nn_items);
        }

        if items.is_empty() {
            let hn_items = self.fetch_hackernews(limit);
            items.extend(hn_items);
        }

        Ok(items)
    }

    fn fetch_sources(&self, sources: &[String], limit: usize) -> Result<Vec<RawNewsItem>, String> {
        let mut items = Vec::new();

        if sources.iter().any(|s| s == "redis") {
            if let Some(ref redis) = self.redis_provider {
                items.extend(redis.fetch_items(limit));
            }
        }

        let non_redis_sources: Vec<&String> = sources.iter().filter(|&s| s != "redis").collect();
        for &s in &non_redis_sources {
            if let Some(ref nn_url) = self.newsnow_url {
                let nn_items = self.fetch_newsnow_source(nn_url, s, limit);
                items.extend(nn_items);
            }
        }

        if items.is_empty() && (sources.is_empty() || sources.iter().any(|s| s == "hackernews" || s == "all")) {
            items.extend(self.fetch_hackernews(limit));
        }

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetcher_instantiation() {
        let fetcher = LiveFeedFetcher::new(None);
        assert!(fetcher.newsnow_url.is_none());
        assert!(fetcher.redis_provider.is_none());
    }

    #[test]
    fn test_fetch_sources_with_redis_offline() {
        let redis = RedisFeedProvider::new("redis://127.0.0.1:65534", "test:key", "stream");
        let fetcher = LiveFeedFetcher::new(None).with_redis(redis);
        let items = fetcher.fetch_sources(&["redis".to_string()], 5).unwrap();
        assert!(items.is_empty());
    }
}
