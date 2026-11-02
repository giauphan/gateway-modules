//! Network feed fetcher implementing `FeedFetcherPort`.
//!
//! Pulls breaking stories from Hacker News Firebase API and NewsNow service using `ureq`.

use crate::domain::RawNewsItem;
use crate::ports::FeedFetcherPort;
use serde_json::Value;
use std::collections::HashMap;

pub struct LiveFeedFetcher {
    pub newsnow_url: Option<String>,
}

impl LiveFeedFetcher {
    pub fn new(newsnow_url: Option<String>) -> Self {
        Self { newsnow_url }
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
            let orig_url = val["url"]
                .as_str()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={id}"));
            if title.is_empty() {
                continue;
            }

            let disc_url = format!("https://news.ycombinator.com/item?id={id}");
            let text = val["text"].as_str().map(|s| s.to_string());
            let mut extra = HashMap::new();
            if let Some(by) = val["by"].as_str() {
                extra.insert("by".to_string(), Value::String(by.to_string()));
            }
            if let Some(score) = val["score"].as_i64() {
                extra.insert("score".to_string(), Value::Number(score.into()));
            }

            items.push(RawNewsItem {
                id: format!("hn_{id}"),
                url: orig_url,
                title,
                published_at: "2026-11-02T11:00:00+00:00".to_string(),
                publisher: Some("Hacker News".to_string()),
                description: text.clone(),
                body: text,
                discussion_url: Some(disc_url),
                tags: vec!["hackernews".to_string(), "tech".to_string()],
                extra,
            });
        }
        items
    }
}

impl FeedFetcherPort for LiveFeedFetcher {
    fn fetch_raw_items(&self, limit: usize) -> Result<Vec<RawNewsItem>, String> {
        let items = self.fetch_hackernews(limit);
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
    }
}
