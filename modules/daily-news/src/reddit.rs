//! Reddit live feed provider implementing Clean Architecture outbound ports.
//!
//! Ingests technical and community discussions from Reddit with support for
//! configurable subreddits, OAuth client credentials, and public endpoints.

use crate::domain::{extract_domain, format_utc_timestamp, RawNewsItem};
use serde_json::Value;
use std::collections::HashMap;

const DEFAULT_REDDIT_USER_AGENT: &str = concat!(
    "ToolcaseGateway-DailyNews/",
    env!("CARGO_PKG_VERSION"),
    " (by /u/toolcase)"
);

#[derive(Debug, Clone)]
pub struct RedditFeedProvider {
    pub subreddits: Vec<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub user_agent: String,
    pub base_url: Option<String>,
}

impl RedditFeedProvider {
    pub fn new(subreddits: Vec<String>) -> Self {
        Self {
            subreddits,
            client_id: None,
            client_secret: None,
            user_agent: DEFAULT_REDDIT_USER_AGENT.to_string(),
            base_url: None,
        }
    }

    pub fn with_credentials(
        mut self,
        client_id: Option<String>,
        client_secret: Option<String>,
    ) -> Self {
        self.client_id = client_id.filter(|s| !s.trim().is_empty());
        self.client_secret = client_secret.filter(|s| !s.trim().is_empty());
        self
    }

    pub fn with_base_url(mut self, base_url: Option<String>) -> Self {
        self.base_url = base_url.filter(|s| !s.trim().is_empty());
        self
    }

    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        if !user_agent.trim().is_empty() {
            self.user_agent = user_agent;
        }
        self
    }

    pub fn fetch_posts(&self, limit: usize) -> Vec<RawNewsItem> {
        if self.subreddits.is_empty() {
            return Vec::new();
        }

        let per_sub_limit = (limit / self.subreddits.len()).clamp(3, 25);
        let mut items = Vec::new();

        for sub in &self.subreddits {
            let sub_items = self.fetch_subreddit(sub, per_sub_limit);
            items.extend(sub_items);
            if items.len() >= limit {
                break;
            }
        }

        items.truncate(limit);
        items
    }

    pub fn fetch_subreddit(&self, subreddit: &str, limit: usize) -> Vec<RawNewsItem> {
        let clean_sub = subreddit.trim().trim_start_matches("r/");
        if clean_sub.is_empty() {
            return Vec::new();
        }

        if let (Some(cid), Some(sec)) = (&self.client_id, &self.client_secret) {
            if let Ok(auth_items) = self.fetch_authenticated_subreddit(cid, sec, clean_sub, limit) {
                if !auth_items.is_empty() {
                    return auth_items;
                }
            }
        }

        let public_items = self.fetch_public_subreddit(clean_sub, limit);
        if !public_items.is_empty() {
            return public_items;
        }

        crate::arctic::fetch_arctic_subreddit_with_base(
            self.base_url.as_deref(),
            clean_sub,
            limit,
            Some(&self.user_agent),
        )
    }

    fn fetch_authenticated_subreddit(
        &self,
        client_id: &str,
        client_secret: &str,
        subreddit: &str,
        limit: usize,
    ) -> Result<Vec<RawNewsItem>, String> {
        let token_url = "https://www.reddit.com/api/v1/access_token";
        let auth_hdr = format!(
            "Basic {}",
            simple_base64_encode(format!("{client_id}:{client_secret}").as_bytes())
        );

        let token_resp: Value = crate::arctic::http_agent()
            .post(token_url)
            .header("Authorization", &auth_hdr)
            .header("User-Agent", &self.user_agent)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send("grant_type=client_credentials")
            .map_err(|e| e.to_string())?
            .into_body()
            .read_json()
            .map_err(|e| e.to_string())?;

        let access_token = token_resp["access_token"]
            .as_str()
            .ok_or_else(|| "Missing access_token in Reddit OAuth response".to_string())?;

        let oauth_url = format!("https://oauth.reddit.com/r/{subreddit}/hot?limit={limit}");
        let val: Value = crate::arctic::http_agent()
            .get(&oauth_url)
            .header("Authorization", &format!("Bearer {access_token}"))
            .header("User-Agent", &self.user_agent)
            .call()
            .map_err(|e| e.to_string())?
            .into_body()
            .read_json()
            .map_err(|e| e.to_string())?;

        Ok(parse_reddit_json(&val, subreddit))
    }

    fn fetch_public_subreddit(&self, subreddit: &str, limit: usize) -> Vec<RawNewsItem> {
        let default_host = "https://www.reddit.com".to_string();
        let base = self.base_url.as_ref().unwrap_or(&default_host);
        let endpoint = format!(
            "{}/r/{subreddit}/hot.json?limit={limit}",
            base.trim_end_matches('/')
        );

        let val: Value = match crate::arctic::http_agent()
            .get(&endpoint)
            .header("Accept", "application/json")
            .header("User-Agent", &self.user_agent)
            .call()
        {
            Ok(res) => res.into_body().read_json().unwrap_or_default(),
            Err(_) => return Vec::new(),
        };

        parse_reddit_json(&val, subreddit)
    }
}

pub fn parse_reddit_json(val: &Value, subreddit: &str) -> Vec<RawNewsItem> {
    let children = match val["data"]["children"].as_array() {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut items = Vec::new();
    for child in children {
        let d = &child["data"];
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
        let score = d["score"].as_i64().unwrap_or(0);
        let comments = d["num_comments"].as_i64().unwrap_or(0);
        let selftext = d["selftext"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        let summary = selftext.unwrap_or_else(|| {
            let by_str = author.as_deref().unwrap_or("community");
            format!(
                "{title} — posted in r/{subreddit} by u/{by_str} ({publisher}, {score} points, {comments} comments)."
            )
        });

        let mut extra = HashMap::new();
        if let Some(ref a) = author {
            extra.insert("author".to_string(), Value::String(a.clone()));
        }
        extra.insert("score".to_string(), Value::Number(score.into()));
        extra.insert("comments".to_string(), Value::Number(comments.into()));
        if let Some(ratio) = d["upvote_ratio"].as_f64() {
            if let Some(n) = serde_json::Number::from_f64(ratio) {
                extra.insert("upvote_ratio".to_string(), Value::Number(n));
            }
        }
        extra.insert(
            "subreddit".to_string(),
            Value::String(subreddit.to_string()),
        );
        extra.insert(
            "discussion_url".to_string(),
            Value::String(disc_url.clone()),
        );

        let mut item = RawNewsItem::new(
            format!("reddit_{subreddit}_{id}"),
            dest_url,
            title.to_string(),
            pub_timestamp,
        );
        item.publisher = Some(publisher);
        item.author = author;
        item.description = Some(summary.clone());
        item.body = Some(summary.clone());
        item.discussion_url = Some(disc_url);
        item.content_kind = Some("news".to_string());
        item.evidence.hook = Some(title.to_string());
        item.evidence.evidence_excerpt = Some(summary.chars().take(280).collect());
        item.tags = vec!["reddit".to_string(), subreddit.to_lowercase()];
        item.extra = extra;

        items.push(item);
    }

    items
}

fn simple_base64_encode(input: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as usize;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as usize
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as usize
        } else {
            0
        };

        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARSET[(triple >> 18) & 0x3F] as char);
        out.push(CHARSET[(triple >> 12) & 0x3F] as char);

        if i + 1 < input.len() {
            out.push(CHARSET[(triple >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }

        if i + 2 < input.len() {
            out.push(CHARSET[triple & 0x3F] as char);
        } else {
            out.push('=');
        }

        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_reddit_json_external_link() {
        let payload = json!({
            "data": {
                "children": [{
                    "data": {
                        "id": "1gabcde",
                        "title": "vLLM 0.6.4 Released with FP8 KV Cache",
                        "url": "https://github.com/vllm-project/vllm/releases/tag/v0.6.4",
                        "permalink": "/r/LocalLLaMA/comments/1gabcde/vllm_064_released/",
                        "author": "llm_researcher",
                        "selftext": "New release features memory optimization and speedups.",
                        "created_utc": 1793618400,
                        "score": 342,
                        "num_comments": 48,
                        "is_self": false
                    }
                }]
            }
        });

        let items = parse_reddit_json(&payload, "LocalLLaMA");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "reddit_LocalLLaMA_1gabcde");
        assert_eq!(item.title, "vLLM 0.6.4 Released with FP8 KV Cache");
        assert_eq!(
            item.url,
            "https://github.com/vllm-project/vllm/releases/tag/v0.6.4"
        );
        assert_eq!(item.publisher.as_deref(), Some("github.com"));
        assert_eq!(
            item.discussion_url.as_deref(),
            Some("https://www.reddit.com/r/LocalLLaMA/comments/1gabcde/vllm_064_released/")
        );
        assert_eq!(item.author.as_deref(), Some("llm_researcher"));
        assert_eq!(item.tags, vec!["reddit", "localllama"]);
        assert_eq!(item.extra["score"], 342);
        assert_eq!(item.extra["comments"], 48);
    }

    #[test]
    fn test_parse_reddit_json_self_post() {
        let payload = json!({
            "data": {
                "children": [{
                    "data": {
                        "id": "2hxyz",
                        "title": "Discussion: What is your preferred quant for 70B models?",
                        "url": "https://www.reddit.com/r/LocalLLaMA/comments/2hxyz/discussion/",
                        "permalink": "/r/LocalLLaMA/comments/2hxyz/discussion/",
                        "author": "quant_fan",
                        "selftext": "Looking at EXL2 vs GGUF Q4_K_M for latency.",
                        "created_utc": 1793618400,
                        "score": 85,
                        "num_comments": 62,
                        "is_self": true
                    }
                }]
            }
        });

        let items = parse_reddit_json(&payload, "LocalLLaMA");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "reddit_LocalLLaMA_2hxyz");
        assert_eq!(
            item.url,
            "https://www.reddit.com/r/LocalLLaMA/comments/2hxyz/discussion/"
        );
        assert_eq!(item.publisher.as_deref(), Some("reddit.com"));
        assert_eq!(
            item.discussion_url.as_deref(),
            Some("https://www.reddit.com/r/LocalLLaMA/comments/2hxyz/discussion/")
        );
    }

    #[test]
    fn test_provider_offline_resilience() {
        let provider = RedditFeedProvider::new(vec!["LocalLLaMA".to_string()])
            .with_base_url(Some("http://127.0.0.1:65534".to_string()));
        let items = provider.fetch_posts(5);
        assert!(items.is_empty());
    }

    #[test]
    fn test_provider_builder_and_defaults() {
        let provider = RedditFeedProvider::new(vec![
            "artificial".to_string(),
            "MachineLearning".to_string(),
        ])
        .with_credentials(Some("client123".into()), Some("secret456".into()))
        .with_user_agent("CustomUA/1.0".into());

        assert_eq!(provider.subreddits.len(), 2);
        assert_eq!(provider.client_id.as_deref(), Some("client123"));
        assert_eq!(provider.client_secret.as_deref(), Some("secret456"));
        assert_eq!(provider.user_agent, "CustomUA/1.0");
    }

    #[test]
    fn test_simple_base64_encode() {
        assert_eq!(simple_base64_encode(b""), "");
        assert_eq!(simple_base64_encode(b"f"), "Zg==");
        assert_eq!(simple_base64_encode(b"fo"), "Zm8=");
        assert_eq!(simple_base64_encode(b"foo"), "Zm9v");
        assert_eq!(
            simple_base64_encode(b"client:secret"),
            "Y2xpZW50OnNlY3JldA=="
        );
    }
}
