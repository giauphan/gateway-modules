//! Reddit Discussion context and thread inspector.
//!
//! Provides on-demand thread inspection (comments tree, authors, upvotes,
//! discussion hierarchy) using Arctic Shift's zero-key APIs.

use crate::domain::{extract_hn_item_id, format_utc_timestamp};
use crate::hn::fetch_hn_discussion_thread;
use crate::ports::DiscussionFetcherPort;
use serde_json::Value;

const DEFAULT_ARCTIC_BASE_URL: &str = "https://arctic-shift.photon-reddit.com";
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Default)]
pub struct LiveDiscussionFetcher {
    pub base_url: Option<String>,
    pub user_agent: Option<String>,
}

impl LiveDiscussionFetcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DiscussionFetcherPort for LiveDiscussionFetcher {
    fn fetch_thread(
        &self,
        url_or_id: &str,
        limit: Option<usize>,
    ) -> Result<DiscussionThread, String> {
        fetch_discussion_thread_with_base(
            self.base_url.as_deref(),
            url_or_id,
            limit,
            self.user_agent.as_deref(),
        )
    }
}

pub use crate::domain::{DiscussionComment, DiscussionThread};

/// Extracts Reddit post ID from full URL, shortlink, or prefixed ID.
pub fn extract_post_id(url_or_id: &str) -> Option<String> {
    let s = url_or_id.trim();
    if s.is_empty() {
        return None;
    }

    if let Some(rest) = s.strip_prefix("reddit_") {
        if let Some((_, post_id)) = rest.split_once('_') {
            return Some(post_id.to_string());
        }
    }

    if let Some(rest) = s.strip_prefix("t3_") {
        return Some(rest.to_string());
    }

    if s.contains("/comments/") {
        if let Some(pos) = s.find("/comments/") {
            let rest = &s[pos + "/comments/".len()..];
            let id = rest.split('/').next().unwrap_or("").trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }

    if s.contains("redd.it/") {
        if let Some(pos) = s.find("redd.it/") {
            let rest = &s[pos + "redd.it/".len()..];
            let id = rest.split('/').next().unwrap_or("").trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }

    if !s.contains('/') && !s.contains('?') && !s.contains('&') && s.len() <= 20 {
        return Some(s.to_string());
    }

    None
}

/// Fetch discussion thread from Arctic Shift on demand.
pub fn fetch_discussion_thread(
    url_or_id: &str,
    limit: Option<usize>,
    user_agent: Option<&str>,
) -> Result<DiscussionThread, String> {
    fetch_discussion_thread_with_base(None, url_or_id, limit, user_agent)
}

/// Fetch discussion thread with optional custom base URL for testing / mocking.
pub fn fetch_discussion_thread_with_base(
    base_url: Option<&str>,
    url_or_id: &str,
    limit: Option<usize>,
    user_agent: Option<&str>,
) -> Result<DiscussionThread, String> {
    if let Some(hn_id) = extract_hn_item_id(url_or_id) {
        return fetch_hn_discussion_thread(base_url, hn_id, limit);
    }

    let post_id = extract_post_id(url_or_id)
        .ok_or_else(|| format!("Could not extract post ID from '{url_or_id}'"))?;

    let base = base_url.unwrap_or(DEFAULT_ARCTIC_BASE_URL);
    let ua = user_agent.unwrap_or(DEFAULT_USER_AGENT);
    let max_comments = limit.unwrap_or(50);

    // 1. Fetch post metadata
    let post_endpoint = format!("{}/api/posts/ids?ids={post_id}", base.trim_end_matches('/'));
    let post_val: Value = crate::arctic::http_agent()
        .get(&post_endpoint)
        .header("User-Agent", ua)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("HTTP request to fetch post metadata failed: {e}"))?
        .into_body()
        .read_json()
        .unwrap_or_default();

    let mut thread = parse_post_metadata(&post_val, &post_id);

    // 2. Fetch comments tree
    let comments_endpoint = format!(
        "{}/api/comments/tree?link_id=t3_{post_id}",
        base.trim_end_matches('/')
    );
    let comments_val: Value = match crate::arctic::http_agent()
        .get(&comments_endpoint)
        .header("User-Agent", ua)
        .header("Accept", "application/json")
        .call()
    {
        Ok(res) => res.into_body().read_json().unwrap_or_default(),
        Err(_) => Value::Null,
    };

    thread.comments = parse_comments_tree(&comments_val, max_comments);
    Ok(thread)
}

pub fn parse_post_metadata(val: &Value, post_id: &str) -> DiscussionThread {
    if let Some(post) = val["data"].as_array().and_then(|arr| arr.first()) {
        let subreddit = post["subreddit"].as_str().map(str::to_string);
        let title = post["title"].as_str().map(str::to_string);
        let author = post["author"].as_str().map(str::to_string);
        let permalink = post["permalink"]
            .as_str()
            .map(|p| {
                if p.starts_with('/') {
                    format!("https://www.reddit.com{p}")
                } else {
                    format!("https://www.reddit.com/{p}")
                }
            })
            .unwrap_or_else(|| format!("https://www.reddit.com/comments/{post_id}"));
        let url = post["url"].as_str().map(str::to_string);
        let selftext = post["selftext"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        let score = post["score"].as_i64().unwrap_or(0);
        let num_comments = post["num_comments"].as_i64().unwrap_or(0);

        DiscussionThread {
            id: post_id.to_string(),
            subreddit,
            title,
            author,
            permalink,
            url,
            selftext,
            score,
            num_comments,
            comments: Vec::new(),
        }
    } else {
        DiscussionThread {
            id: post_id.to_string(),
            subreddit: None,
            title: None,
            author: None,
            permalink: format!("https://www.reddit.com/comments/{post_id}"),
            url: None,
            selftext: None,
            score: 0,
            num_comments: 0,
            comments: Vec::new(),
        }
    }
}

pub fn parse_comments_tree(val: &Value, max_comments: usize) -> Vec<DiscussionComment> {
    let arr = match val["data"].as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut comments = Vec::new();
    for item in arr {
        if comments.len() >= max_comments {
            break;
        }
        if let Some(c) = parse_comment_node(item, max_comments - comments.len()) {
            comments.push(c);
        }
    }
    comments
}

fn parse_comment_node(node: &Value, max_depth_comments: usize) -> Option<DiscussionComment> {
    let d = if node["kind"].as_str() == Some("t1") && node["data"].is_object() {
        &node["data"]
    } else if node.is_object() {
        node
    } else {
        return None;
    };

    let id = d["id"].as_str()?.to_string();
    let body = d["body"].as_str()?.to_string();
    if body == "[deleted]" || body == "[removed]" {
        return None;
    }

    let author = d["author"].as_str().map(str::to_string);
    let score = d["score"].as_i64().unwrap_or(0);
    let created_utc = d["created_utc"]
        .as_u64()
        .or_else(|| d["created_utc"].as_f64().map(|f| f as u64))
        .unwrap_or(0);
    let created_at = format_utc_timestamp(created_utc);
    let permalink = d["permalink"].as_str().map(|p| {
        if p.starts_with('/') {
            format!("https://www.reddit.com{p}")
        } else {
            format!("https://www.reddit.com/{p}")
        }
    });

    let mut replies = Vec::new();
    if max_depth_comments > 1 {
        if let Some(children) = d["replies"]["data"]["children"].as_array() {
            for child in children {
                if replies.len() >= max_depth_comments - 1 {
                    break;
                }
                if let Some(rc) = parse_comment_node(child, max_depth_comments - 1 - replies.len())
                {
                    replies.push(rc);
                }
            }
        }
    }

    Some(DiscussionComment {
        id,
        author,
        body,
        score,
        created_utc,
        created_at,
        permalink,
        replies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_post_id_various_formats() {
        assert_eq!(
            extract_post_id("https://www.reddit.com/r/LocalLLaMA/comments/1xmitzx/alibabas_qwen/"),
            Some("1xmitzx".to_string())
        );
        assert_eq!(
            extract_post_id("https://reddit.com/comments/1xmitzx/"),
            Some("1xmitzx".to_string())
        );
        assert_eq!(
            extract_post_id("https://redd.it/1xmitzx"),
            Some("1xmitzx".to_string())
        );
        assert_eq!(
            extract_post_id("reddit_LocalLLaMA_1xmitzx"),
            Some("1xmitzx".to_string())
        );
        assert_eq!(extract_post_id("t3_1xmitzx"), Some("1xmitzx".to_string()));
        assert_eq!(extract_post_id("1xmitzx"), Some("1xmitzx".to_string()));
        assert_eq!(extract_post_id(""), None);
        assert_eq!(extract_post_id("   "), None);
    }

    #[test]
    fn test_parse_comments_tree_with_replies() {
        let tree_json = json!({
            "data": [
                {
                    "kind": "t1",
                    "data": {
                        "id": "c1",
                        "author": "user1",
                        "body": "First top-level comment",
                        "score": 42,
                        "created_utc": 1793680000,
                        "permalink": "/r/LocalLLaMA/comments/xyz/post/c1/",
                        "replies": {
                            "kind": "Listing",
                            "data": {
                                "children": [
                                    {
                                        "kind": "t1",
                                        "data": {
                                            "id": "c1_reply1",
                                            "author": "user2",
                                            "body": "Reply to first comment",
                                            "score": 10,
                                            "created_utc": 1793680100,
                                            "permalink": "/r/LocalLLaMA/comments/xyz/post/c1_reply1/",
                                            "replies": ""
                                        }
                                    }
                                ]
                            }
                        }
                    }
                },
                {
                    "kind": "t1",
                    "data": {
                        "id": "c2",
                        "author": "user3",
                        "body": "[deleted]",
                        "score": -5,
                        "created_utc": 1793680200,
                        "replies": ""
                    }
                }
            ]
        });

        let comments = parse_comments_tree(&tree_json, 10);
        // c2 is [deleted] so it should be skipped
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, "c1");
        assert_eq!(comments[0].author.as_deref(), Some("user1"));
        assert_eq!(comments[0].body, "First top-level comment");
        assert_eq!(comments[0].score, 42);
        assert_eq!(comments[0].replies.len(), 1);
        assert_eq!(comments[0].replies[0].id, "c1_reply1");
        assert_eq!(comments[0].replies[0].author.as_deref(), Some("user2"));
    }

    #[test]
    fn test_parse_post_metadata() {
        let post_json = json!({
            "data": [
                {
                    "id": "1xmitzx",
                    "title": "Leaked models",
                    "author": "dev_hero",
                    "subreddit": "LocalLLaMA",
                    "score": 150,
                    "num_comments": 25,
                    "permalink": "/r/LocalLLaMA/comments/1xmitzx/leaked_models/",
                    "url": "https://github.com/repo",
                    "selftext": "Details here"
                }
            ]
        });

        let thread = parse_post_metadata(&post_json, "1xmitzx");
        assert_eq!(thread.id, "1xmitzx");
        assert_eq!(thread.title.as_deref(), Some("Leaked models"));
        assert_eq!(thread.author.as_deref(), Some("dev_hero"));
        assert_eq!(thread.subreddit.as_deref(), Some("LocalLLaMA"));
        assert_eq!(thread.score, 150);
        assert_eq!(thread.num_comments, 25);
        assert_eq!(thread.url.as_deref(), Some("https://github.com/repo"));
        assert_eq!(thread.selftext.as_deref(), Some("Details here"));
    }
}
