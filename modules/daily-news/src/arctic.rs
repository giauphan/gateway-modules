//! Arctic Shift Reddit archive and search provider.
//!
//! Provides a zero-key fallback for Reddit content when direct public endpoints
//! are blocked (e.g. HTTP 403 on cloud datacenter IP ranges).

use crate::domain::{extract_domain, format_utc_timestamp, RawNewsItem};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use ureq::Agent;

const DEFAULT_ARCTIC_BASE_URL: &str = "https://arctic-shift.photon-reddit.com";
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

pub fn http_agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        Agent::config_builder()
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .new_agent()
    })
}

pub fn fetch_arctic_subreddit(
    subreddit: &str,
    limit: usize,
    user_agent: Option<&str>,
) -> Vec<RawNewsItem> {
    fetch_arctic_subreddit_with_base(None, subreddit, limit, user_agent)
}

pub fn fetch_arctic_subreddit_with_base(
    base_url: Option<&str>,
    subreddit: &str,
    limit: usize,
    user_agent: Option<&str>,
) -> Vec<RawNewsItem> {
    let clean_sub = subreddit.trim().trim_start_matches("r/");
    if clean_sub.is_empty() {
        return Vec::new();
    }

    let base = base_url.unwrap_or(DEFAULT_ARCTIC_BASE_URL);
    let ua = user_agent.unwrap_or(DEFAULT_USER_AGENT);
    let endpoint = format!(
        "{}/api/posts/search?subreddit={clean_sub}&limit={limit}",
        base.trim_end_matches('/')
    );

    let val: Value = match http_agent()
        .get(&endpoint)
        .header("User-Agent", ua)
        .header("Accept", "application/json")
        .call()
    {
        Ok(res) => res.into_body().read_json().unwrap_or_default(),
        Err(_) => return Vec::new(),
    };

    parse_arctic_json(&val, clean_sub)
}

pub fn parse_arctic_json(val: &Value, subreddit: &str) -> Vec<RawNewsItem> {
    let posts = match val["data"].as_array() {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut items = Vec::new();
    for d in posts {
        let id = d["id"].as_str().unwrap_or("").trim();
        let title = d["title"].as_str().unwrap_or("").trim();
        if id.is_empty() || title.is_empty() {
            continue;
        }

        let is_self = d["is_self"].as_bool().unwrap_or(false);
        let permalink = d["permalink"].as_str().unwrap_or("");
        let disc_url = if permalink.starts_with('/') {
            format!("https://www.reddit.com{permalink}")
        } else {
            format!("https://www.reddit.com/{permalink}")
        };

        let raw_url = d["url"].as_str().unwrap_or("").trim();
        let dest_url = if is_self
            || raw_url.is_empty()
            || raw_url.starts_with('/')
            || raw_url.contains("reddit.com/r/")
        {
            disc_url.clone()
        } else {
            raw_url.to_string()
        };

        let publisher = if is_self {
            "reddit.com".to_string()
        } else {
            extract_domain(&dest_url)
        };

        let created_utc = d["created_utc"]
            .as_u64()
            .or_else(|| d["created_utc"].as_f64().map(|f| f as u64))
            .unwrap_or(1793618000);

        let pub_timestamp = format_utc_timestamp(created_utc);
        let author = d["author"].as_str().map(str::to_string);
        let selftext = d["selftext"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        let mut extra = HashMap::new();
        if let Some(ref a) = author {
            extra.insert("author".to_string(), Value::String(a.clone()));
        }
        if let Some(score) = d["score"].as_i64() {
            extra.insert("score".to_string(), Value::Number(score.into()));
        }
        if let Some(comments) = d["num_comments"].as_i64() {
            extra.insert("comments".to_string(), Value::Number(comments.into()));
        }
        extra.insert(
            "subreddit".to_string(),
            Value::String(subreddit.to_string()),
        );
        extra.insert(
            "discussion_url".to_string(),
            Value::String(disc_url.clone()),
        );
        extra.insert(
            "provider".to_string(),
            Value::String("arctic_shift".to_string()),
        );

        let mut item = RawNewsItem::new(
            format!("reddit_{subreddit}_{id}"),
            dest_url,
            title.to_string(),
            pub_timestamp,
        );
        item.publisher = Some(publisher);
        item.author = author;
        item.description = selftext.clone();
        item.body = selftext;
        item.discussion_url = Some(disc_url);
        item.tags = vec!["reddit".to_string(), subreddit.to_lowercase()];
        item.extra = extra;

        items.push(item);
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_arctic_json_link_and_self_post() {
        let payload = json!({
            "data": [
                {
                    "id": "post_1",
                    "title": "OpenAI Introduces Agent Framework v2",
                    "url": "https://openai.com/index/agent-framework-v2",
                    "permalink": "/r/LocalLLaMA/comments/post_1/openai_introduces/",
                    "author": "frontier_dev",
                    "selftext": "Key architecture details on token caching and tool execution.",
                    "created_utc": 1793685000,
                    "score": 45,
                    "num_comments": 12,
                    "is_self": false
                },
                {
                    "id": "post_2",
                    "title": "Community Benchmark on RTX 4090 Quantization",
                    "url": "https://www.reddit.com/r/LocalLLaMA/comments/post_2/benchmark/",
                    "permalink": "/r/LocalLLaMA/comments/post_2/benchmark/",
                    "author": "gpu_bench",
                    "selftext": "Comparing EXL2 vs GGUF tokens/sec throughput.",
                    "created_utc": 1793685100,
                    "score": 120,
                    "num_comments": 34,
                    "is_self": true
                }
            ]
        });

        let items = parse_arctic_json(&payload, "LocalLLaMA");
        assert_eq!(items.len(), 2);

        // 1. External link
        let item1 = &items[0];
        assert_eq!(item1.id, "reddit_LocalLLaMA_post_1");
        assert_eq!(item1.title, "OpenAI Introduces Agent Framework v2");
        assert_eq!(item1.url, "https://openai.com/index/agent-framework-v2");
        assert_eq!(item1.publisher.as_deref(), Some("openai.com"));
        assert_eq!(
            item1.discussion_url.as_deref(),
            Some("https://www.reddit.com/r/LocalLLaMA/comments/post_1/openai_introduces/")
        );
        assert_eq!(item1.extra["provider"], "arctic_shift");

        // 2. Self post
        let item2 = &items[1];
        assert_eq!(item2.id, "reddit_LocalLLaMA_post_2");
        assert_eq!(
            item2.url,
            "https://www.reddit.com/r/LocalLLaMA/comments/post_2/benchmark/"
        );
        assert_eq!(item2.publisher.as_deref(), Some("reddit.com"));
    }

    #[test]
    fn test_parse_arctic_json_empty_and_malformed() {
        let empty_payload = json!({ "data": [] });
        let items = parse_arctic_json(&empty_payload, "LocalLLaMA");
        assert!(items.is_empty());

        let malformed = json!({ "error": "Rate limit exceeded" });
        let items = parse_arctic_json(&malformed, "LocalLLaMA");
        assert!(items.is_empty());
    }

    #[test]
    fn test_fetch_arctic_empty_subreddit() {
        let items = fetch_arctic_subreddit("", 5, None);
        assert!(items.is_empty());
    }
}
