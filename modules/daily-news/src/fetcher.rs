//! Network feed fetcher implementing `FeedFetcherPort`.
//!
//! Pulls breaking stories from Hacker News, NewsNow, and Redis providers using `ureq`.

use crate::domain::{
    clean_html_text, current_utc_timestamp, extract_domain, extract_hn_item_id,
    format_utc_timestamp, RawNewsItem,
};
use crate::hn::{fetch_hackernews_top, fetch_hn_item};
use crate::ports::FeedFetcherPort;
use crate::reddit::RedditFeedProvider;
use crate::redis::RedisFeedProvider;
use serde_json::Value;
use std::collections::HashMap;

pub struct LiveFeedFetcher {
    pub newsnow_url: Option<String>,
    pub redis_provider: Option<RedisFeedProvider>,
    pub reddit_provider: Option<RedditFeedProvider>,
}

impl LiveFeedFetcher {
    pub fn new(newsnow_url: Option<String>) -> Self {
        Self {
            newsnow_url,
            redis_provider: None,
            reddit_provider: None,
        }
    }

    pub fn with_redis(mut self, redis: RedisFeedProvider) -> Self {
        self.redis_provider = Some(redis);
        self
    }

    pub fn with_reddit(mut self, reddit: RedditFeedProvider) -> Self {
        self.reddit_provider = Some(reddit);
        self
    }

    fn fetch_newsnow_source(&self, base_url: &str, source: &str, limit: usize) -> Vec<RawNewsItem> {
        let endpoint = format!("{}/api/s?id={source}", base_url.trim_end_matches('/'));
        let val: Value = match crate::arctic::http_agent()
            .get(&endpoint)
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                concat!("toolcase-daily-news/", env!("CARGO_PKG_VERSION")),
            )
            .call()
        {
            Ok(res) => res.into_body().read_json().unwrap_or_default(),
            Err(_) => return Vec::new(),
        };

        let items = parse_newsnow_items(&val, source, limit);
        if source == "hackernews" {
            items
                .into_iter()
                .map(|item| {
                    if let Some(hn_id) =
                        extract_hn_item_id(&item.id).or_else(|| extract_hn_item_id(&item.url))
                    {
                        if let Some(mut enriched) = fetch_hn_item(hn_id) {
                            enriched.id = item.id;
                            return enriched;
                        }
                    }
                    item
                })
                .collect()
        } else {
            items
        }
    }

    fn fetch_hackernews(&self, limit: usize) -> Vec<RawNewsItem> {
        fetch_hackernews_top(limit)
    }
}

fn resolve_newsnow_timestamp(raw: &Value, root: &Value) -> (String, crate::domain::PublishedAtKind) {
    let candidates = [
        &raw["published_at"],
        &raw["pubDate"],
        &raw["extra"]["date"],
        &root["updatedTime"],
    ];
    for c in candidates {
        if let Some(s) = c.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                if let Ok(num) = trimmed.parse::<u64>() {
                    let secs = if num > 100_000_000_000 {
                        num / 1000
                    } else {
                        num
                    };
                    return (format_utc_timestamp(secs), crate::domain::PublishedAtKind::ArticlePublication);
                }
                return (trimmed.to_string(), crate::domain::PublishedAtKind::ArticlePublication);
            }
        }
        if let Some(num) = c.as_u64() {
            let secs = if num > 100_000_000_000 {
                num / 1000
            } else {
                num
            };
            return (format_utc_timestamp(secs), crate::domain::PublishedAtKind::ArticlePublication);
        }
    }
    (current_utc_timestamp(), crate::domain::PublishedAtKind::FeedObserved)
}

pub fn parse_newsnow_items(val: &Value, source: &str, limit: usize) -> Vec<RawNewsItem> {
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
        let item_id = if let Some(s) = raw["id"].as_str() {
            s.trim().to_string()
        } else if let Some(n) = raw["id"].as_u64() {
            n.to_string()
        } else if let Some(n) = raw["id"].as_i64() {
            n.to_string()
        } else {
            url.clone()
        };

        let publisher = extract_domain(&url);
        let (pub_timestamp, published_at_kind) = resolve_newsnow_timestamp(raw, val);

        let mut extra = HashMap::new();
        if let Some(obj) = raw["extra"].as_object() {
            for (k, v) in obj {
                extra.insert(k.clone(), v.clone());
            }
        }

        let disc_url = raw["discussion_url"]
            .as_str()
            .or_else(|| raw["comments_url"].as_str())
            .or_else(|| raw["extra"]["url"].as_str())
            .map(str::to_string)
            .or_else(|| {
                if url.contains("news.ycombinator.com/item") || url.contains("reddit.com/r/") {
                    Some(url.clone())
                } else {
                    None
                }
            });

        if let Some(ref d) = disc_url {
            extra
                .entry("discussion_url".to_string())
                .or_insert_with(|| Value::String(d.clone()));
        }

        let author = raw["author"]
            .as_str()
            .or_else(|| raw["extra"]["by"].as_str())
            .or_else(|| raw["extra"]["author"].as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let raw_desc = raw["description"]
            .as_str()
            .or_else(|| raw["extra"]["hover"].as_str())
            .or_else(|| raw["extra"]["info"].as_str())
            .map(clean_html_text)
            .filter(|s| !s.is_empty());

        let raw_body = raw["body"]
            .as_str()
            .map(clean_html_text)
            .filter(|s| !s.is_empty());

        let has_full_body = raw_body.is_some();
        let body = raw_body;
        let summary = raw_desc.or_else(|| body.clone());
        let origin = if has_full_body {
            crate::domain::ContentOrigin::Article
        } else {
            crate::domain::ContentOrigin::HeadlineOnly
        };

        let mut item = RawNewsItem::new(
            format!("newsnow_{source}_{item_id}"),
            url,
            title.clone(),
            pub_timestamp,
        );
        item.publisher = Some(publisher);
        item.author = author.clone();
        item.submission_author = author;
        item.description = summary.clone();
        item.body = body;
        item.discussion_url = disc_url;
        item.content_kind = Some("news".to_string());
        item.content_origin = origin;
        item.published_at_kind = published_at_kind;
        item.evidence.hook = Some(title);
        item.hook_origin = Some("raw_headline".to_string());
        if has_full_body {
            item.evidence.evidence_excerpt = summary.map(|s| s.chars().take(280).collect());
        }
        item.tags = vec![source.to_string()];
        item.extra = extra;
        items.push(item);
    }
    items
}

fn interleave_source_items(source_batches: Vec<Vec<RawNewsItem>>) -> Vec<RawNewsItem> {
    let mut result = Vec::new();
    let max_len = source_batches.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..max_len {
        for batch in &source_batches {
            if let Some(item) = batch.get(i) {
                result.push(item.clone());
            }
        }
    }
    result
}

impl FeedFetcherPort for LiveFeedFetcher {
    fn fetch_raw_items(&self, limit: usize) -> Result<Vec<RawNewsItem>, String> {
        let mut batches = Vec::new();
        if let Some(ref redis) = self.redis_provider {
            let redis_items = redis.fetch_items(limit);
            if !redis_items.is_empty() {
                batches.push(redis_items);
            }
        }

        if let Some(ref reddit) = self.reddit_provider {
            let reddit_items = reddit.fetch_posts(limit);
            if !reddit_items.is_empty() {
                batches.push(reddit_items);
            }
        }

        let hn_items = self.fetch_hackernews(limit);
        if !hn_items.is_empty() {
            batches.push(hn_items);
        } else if let Some(ref nn_url) = self.newsnow_url {
            let nn_items = self.fetch_newsnow_source(nn_url, "hackernews", limit);
            if !nn_items.is_empty() {
                batches.push(nn_items);
            }
        }

        Ok(interleave_source_items(batches))
    }

    fn fetch_sources(&self, sources: &[String], limit: usize) -> Result<Vec<RawNewsItem>, String> {
        let mut batches = Vec::new();

        if sources.iter().any(|s| s == "redis") {
            if let Some(ref redis) = self.redis_provider {
                let redis_items = redis.fetch_items(limit);
                if !redis_items.is_empty() {
                    batches.push(redis_items);
                }
            }
        }

        let mut reddit_fetched = false;
        if sources.iter().any(|s| s == "reddit" || s == "all") {
            if let Some(ref reddit) = self.reddit_provider {
                let reddit_items = reddit.fetch_posts(limit);
                if !reddit_items.is_empty() {
                    batches.push(reddit_items);
                    reddit_fetched = true;
                }
            }
        }

        let mut hn_fetched = false;
        if sources.is_empty() || sources.iter().any(|s| s == "hackernews" || s == "all") {
            let hn_items = self.fetch_hackernews(limit);
            if !hn_items.is_empty() {
                batches.push(hn_items);
                hn_fetched = true;
            }
        }

        let non_special_sources: Vec<&String> = sources
            .iter()
            .filter(|&s| {
                s != "redis"
                    && (s != "reddit" || !reddit_fetched)
                    && (s != "hackernews" || !hn_fetched)
            })
            .collect();
        for &s in &non_special_sources {
            if let Some(ref nn_url) = self.newsnow_url {
                let nn_items = self.fetch_newsnow_source(nn_url, s, limit);
                if !nn_items.is_empty() {
                    batches.push(nn_items);
                }
            }
        }

        Ok(interleave_source_items(batches))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hn::parse_hn_item_json;
    use serde_json::json;

    #[test]
    fn test_fetcher_instantiation() {
        let fetcher = LiveFeedFetcher::new(None);
        assert!(fetcher.newsnow_url.is_none());
        assert!(fetcher.redis_provider.is_none());
        assert!(fetcher.reddit_provider.is_none());
    }

    #[test]
    fn test_fetch_sources_with_redis_offline() {
        let redis = RedisFeedProvider::new("redis://127.0.0.1:65534", "test:key", "stream");
        let fetcher = LiveFeedFetcher::new(None).with_redis(redis);
        let items = fetcher.fetch_sources(&["redis".to_string()], 5).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_fetch_sources_with_reddit_offline() {
        let reddit = RedditFeedProvider::new(vec!["LocalLLaMA".to_string()])
            .with_base_url(Some("http://127.0.0.1:65534".to_string()));
        let fetcher = LiveFeedFetcher::new(None).with_reddit(reddit);
        let items = fetcher.fetch_sources(&["reddit".to_string()], 5).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_hn_item_with_comments_and_fallback() {
        let hn_json = json!({
            "id": 50346981,
            "by": "nytimes_dev",
            "score": 142,
            "descendants": 38,
            "time": 1730545200,
            "title": "Show HN: Hacker Times, a New York Times-inspired version of Hacker News",
            "url": "https://hackertimes.com"
        });
        let comments = vec![
            "alice: Love the serif typography and layout density!".to_string(),
            "bob: Works great on mobile Safari.".to_string(),
        ];
        let item = parse_hn_item_json(&hn_json, 50346981, &comments).unwrap();
        assert_eq!(item.id, "hn_50346981");
        assert_eq!(item.author.as_deref(), Some("nytimes_dev"));
        assert_eq!(item.publisher.as_deref(), Some("hackertimes.com"));
        assert_eq!(
            item.discussion_url.as_deref(),
            Some("https://news.ycombinator.com/item?id=50346981")
        );
        assert_eq!(item.content_kind.as_deref(), Some("news"));
        assert!(item.body.is_none());
        assert!(item.description.is_none());
        assert!(item.discussion.is_some());
        let disc = item.discussion.as_ref().unwrap();
        assert_eq!(disc.comments.len(), 2);
        assert!(disc.comments[0].text.contains("Love the serif typography"));
        assert_eq!(item.extra["score"], 142);
        assert_eq!(item.extra["comments"], 38);
    }

    #[test]
    fn test_parse_newsnow_items_populates_all_fields() {
        let payload = json!({
            "updatedTime": 1730548800000u64,
            "items": [{
                "id": 50346981,
                "title": "Show HN: Hacker Times",
                "url": "https://news.ycombinator.com/item?id=50346981",
                "body": "Full article body about Hacker Times.",
                "extra": {
                    "info": "120 points by ny_author",
                    "by": "ny_author"
                }
            }]
        });
        let items = parse_newsnow_items(&payload, "hackernews", 5);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "newsnow_hackernews_50346981");
        assert_eq!(item.author.as_deref(), Some("ny_author"));
        assert_eq!(item.description.as_deref(), Some("120 points by ny_author"));
        assert_eq!(
            item.body.as_deref(),
            Some("Full article body about Hacker Times.")
        );
        assert_eq!(
            item.discussion_url.as_deref(),
            Some("https://news.ycombinator.com/item?id=50346981")
        );
        assert_eq!(item.content_kind.as_deref(), Some("news"));
        assert!(item.evidence.hook.is_some());
        assert!(item.evidence.evidence_excerpt.is_some());
        assert_ne!(item.published_at, "2026-11-02T11:00:00+00:00");
    }
}
