//! Hacker News Firebase API client, comment enrichment, and discussion thread parser.

use crate::discussion::{DiscussionComment, DiscussionThread};
use crate::domain::{clean_html_text, extract_domain, format_utc_timestamp, RawNewsItem};
use serde_json::Value;
use std::collections::HashMap;

pub const DEFAULT_HN_BASE_URL: &str = "https://hacker-news.firebaseio.com/v0";

pub fn fetch_hn_item(id: u64) -> Option<RawNewsItem> {
    fetch_hn_item_with_base(None, id)
}

fn fetch_single_comment(base: &str, kid: u64) -> Option<String> {
    let kid_url = format!("{base}/item/{kid}.json");
    let res = crate::arctic::http_agent().get(&kid_url).call().ok()?;
    let c_val: Value = res.into_body().read_json().ok()?;
    if c_val["deleted"].as_bool() == Some(true) || c_val["dead"].as_bool() == Some(true) {
        return None;
    }
    let raw_text = c_val["text"].as_str()?;
    let cleaned = clean_html_text(raw_text);
    if cleaned.is_empty() {
        return None;
    }
    let by = c_val["by"].as_str().unwrap_or("commenter");
    Some(format!("{by}: {cleaned}"))
}

fn fetch_item_top_comments(base: &str, val: &Value) -> Vec<String> {
    let has_text = val["text"]
        .as_str()
        .map(|s| !clean_html_text(s).is_empty())
        .unwrap_or(false);
    if has_text {
        return Vec::new();
    }
    let Some(kids) = val["kids"].as_array() else {
        return Vec::new();
    };

    let mut top_comments = Vec::with_capacity(2);
    for kid in kids.iter().filter_map(|v| v.as_u64()).take(3) {
        if top_comments.len() >= 2 {
            break;
        }
        if let Some(comment) = fetch_single_comment(base, kid) {
            top_comments.push(comment);
        }
    }
    top_comments
}

pub fn fetch_hn_item_with_base(base_url: Option<&str>, id: u64) -> Option<RawNewsItem> {
    let base = base_url
        .unwrap_or(DEFAULT_HN_BASE_URL)
        .trim_end_matches('/');
    let item_url = format!("{base}/item/{id}.json");
    let val: Value = crate::arctic::http_agent()
        .get(&item_url)
        .call()
        .ok()?
        .into_body()
        .read_json()
        .ok()?;

    let top_comments = fetch_item_top_comments(base, &val);
    parse_hn_item_json(&val, id, &top_comments)
}

pub fn fetch_hackernews_top(limit: usize) -> Vec<RawNewsItem> {
    if limit == 0 {
        return Vec::new();
    }
    let endpoint = format!("{DEFAULT_HN_BASE_URL}/topstories.json");
    let ids: Vec<u64> = match crate::arctic::http_agent().get(&endpoint).call() {
        Ok(res) => res.into_body().read_json().unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let target_ids: Vec<u64> = ids.into_iter().take(limit.saturating_add(5)).collect();
    let mut items = Vec::new();
    for id in target_ids {
        if let Some(item) = fetch_hn_item(id) {
            items.push(item);
            if items.len() >= limit {
                break;
            }
        }
    }
    items.truncate(limit);
    items
}

pub fn parse_hn_item_json(val: &Value, id: u64, top_comments: &[String]) -> Option<RawNewsItem> {
    let title = val["title"].as_str().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return None;
    }

    let is_external = val["url"]
        .as_str()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let disc_url = format!("https://news.ycombinator.com/item?id={id}");
    let orig_url = val["url"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| disc_url.clone());

    let publisher = if is_external {
        extract_domain(&orig_url)
    } else {
        "news.ycombinator.com".to_string()
    };

    let author = val["by"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let (pub_timestamp, pub_kind) = if let Some(t) = val["time"].as_u64().filter(|&t| t > 0) {
        (
            format_utc_timestamp(t),
            crate::domain::PublishedAtKind::HnSubmission,
        )
    } else {
        (
            "1970-01-01T00:00:00+00:00".to_string(),
            crate::domain::PublishedAtKind::Unknown,
        )
    };

    let score = val["score"].as_i64().unwrap_or(0);
    let comments_count = val["descendants"].as_i64().unwrap_or(0);

    let self_text = val["text"]
        .as_str()
        .map(clean_html_text)
        .filter(|s| !s.is_empty());

    let (body, description, excerpt, origin) = if let Some(text) = self_text {
        let exc: String = text.chars().take(280).collect();
        (
            Some(text.clone()),
            Some(text),
            Some(exc),
            crate::domain::ContentOrigin::HnSelfPost,
        )
    } else if is_external {
        (None, None, None, crate::domain::ContentOrigin::HeadlineOnly)
    } else {
        (None, None, None, crate::domain::ContentOrigin::Unknown)
    };

    let discussion = if !top_comments.is_empty() || is_external {
        let comments = top_comments
            .iter()
            .enumerate()
            .map(|(idx, comment_str)| {
                let (auth, text) = if let Some((a, c)) = comment_str.split_once(": ") {
                    (a.trim().to_string(), c.trim().to_string())
                } else {
                    ("community".to_string(), comment_str.trim().to_string())
                };
                crate::domain::DiscussionCommentItem {
                    id: format!("{id}_comment_{idx}"),
                    author: auth,
                    text,
                    permalink: Some(disc_url.clone()),
                    created_at: None,
                }
            })
            .collect();
        Some(crate::domain::DiscussionPayload {
            url: disc_url.clone(),
            role: "community_opinion".to_string(),
            summary: if !top_comments.is_empty() {
                Some(top_comments.join(" | "))
            } else {
                None
            },
            comments,
        })
    } else {
        None
    };

    let mut extra = HashMap::new();
    if let Some(ref by) = author {
        extra.insert("by".to_string(), Value::String(by.clone()));
    }
    extra.insert("score".to_string(), Value::Number(score.into()));
    extra.insert("comments".to_string(), Value::Number(comments_count.into()));
    extra.insert(
        "discussion_url".to_string(),
        Value::String(disc_url.clone()),
    );

    let mut item = RawNewsItem::new(
        format!("hn_{id}"),
        orig_url.clone(),
        title.clone(),
        pub_timestamp,
    );
    item.publisher = Some(publisher);
    item.author = author.clone();
    item.submission_author = author;
    item.article_author = None;
    item.description = description;
    item.body = body;
    item.discussion_url = Some(disc_url);
    item.content_kind = Some("news".to_string());
    item.evidence.hook = Some(title);
    item.hook_origin = Some("raw_headline".to_string());
    item.evidence.evidence_excerpt = excerpt;
    item.discussion = discussion;
    item.content_origin = origin;
    item.published_at_kind = pub_kind;
    if is_external {
        item.primary_source = Some(crate::domain::PrimarySourceEvidence {
            url: orig_url,
            fetch_status: "NOT_FETCHED".to_string(),
            ..Default::default()
        });
    }
    item.tags = vec!["hackernews".to_string(), "tech".to_string()];
    item.extra = extra;
    Some(item)
}

pub fn fetch_hn_discussion_thread(
    base_url: Option<&str>,
    hn_id: u64,
    limit: Option<usize>,
) -> Result<DiscussionThread, String> {
    let base = base_url
        .unwrap_or(DEFAULT_HN_BASE_URL)
        .trim_end_matches('/');
    let item_endpoint = format!("{base}/item/{hn_id}.json");
    let val: Value = crate::arctic::http_agent()
        .get(&item_endpoint)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("HTTP request to fetch Hacker News item failed: {e}"))?
        .into_body()
        .read_json()
        .unwrap_or_default();

    let max_comments = limit.unwrap_or(20).min(30);
    let mut thread = parse_hn_story_metadata(&val, hn_id);
    if let Some(kids) = val["kids"].as_array() {
        for kid in kids.iter().filter_map(|v| v.as_u64()).take(max_comments) {
            if let Some(comment) = fetch_hn_comment_node(base, kid, 2) {
                thread.comments.push(comment);
            }
        }
    }
    Ok(thread)
}

pub fn parse_hn_story_metadata(val: &Value, hn_id: u64) -> DiscussionThread {
    let permalink = format!("https://news.ycombinator.com/item?id={hn_id}");
    let title = val["title"].as_str().map(str::to_string);
    let author = val["by"].as_str().map(str::to_string);
    let url = val["url"]
        .as_str()
        .map(str::to_string)
        .or_else(|| Some(permalink.clone()));
    let selftext = val["text"]
        .as_str()
        .map(clean_html_text)
        .filter(|s| !s.is_empty());
    let score = val["score"].as_i64().unwrap_or(0);
    let num_comments = val["descendants"].as_i64().unwrap_or(0);

    DiscussionThread {
        id: hn_id.to_string(),
        subreddit: Some("hackernews".to_string()),
        title,
        author,
        permalink,
        url,
        selftext,
        score,
        num_comments,
        comments: Vec::new(),
    }
}

fn fetch_hn_comment_node(base: &str, id: u64, depth: usize) -> Option<DiscussionComment> {
    let endpoint = format!("{base}/item/{id}.json");
    let val: Value = crate::arctic::http_agent()
        .get(&endpoint)
        .header("Accept", "application/json")
        .call()
        .ok()?
        .into_body()
        .read_json()
        .ok()?;

    if val["deleted"].as_bool() == Some(true) || val["dead"].as_bool() == Some(true) {
        return None;
    }
    let raw_text = val["text"].as_str()?;
    let body = clean_html_text(raw_text);
    if body.is_empty() {
        return None;
    }
    let author = val["by"].as_str().map(str::to_string);
    let created_utc = val["time"].as_u64().unwrap_or(0);
    let created_at = format_utc_timestamp(created_utc);
    let permalink = Some(format!("https://news.ycombinator.com/item?id={id}"));

    let mut replies = Vec::new();
    if depth > 1 {
        if let Some(kids) = val["kids"].as_array() {
            for kid in kids.iter().filter_map(|v| v.as_u64()).take(3) {
                if let Some(reply) = fetch_hn_comment_node(base, kid, depth - 1) {
                    replies.push(reply);
                }
            }
        }
    }

    Some(DiscussionComment {
        id: id.to_string(),
        author,
        body,
        score: val["score"].as_i64().unwrap_or(1),
        created_utc,
        created_at,
        permalink,
        replies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::extract_hn_item_id;
    use serde_json::json;

    #[test]
    fn test_extract_hn_item_id_formats() {
        assert_eq!(extract_hn_item_id("hn_50346981"), Some(50346981));
        assert_eq!(
            extract_hn_item_id("newsnow_hackernews_50346981"),
            Some(50346981)
        );
        assert_eq!(
            extract_hn_item_id("https://news.ycombinator.com/item?id=50346981"),
            Some(50346981)
        );
        assert_eq!(
            extract_hn_item_id("https://www.reddit.com/r/LocalLLaMA/comments/abc/"),
            None
        );
    }

    #[test]
    fn test_parse_hn_story_metadata() {
        let val = json!({
            "id": 50346981,
            "title": "Show HN: Hacker Times",
            "by": "ht_author",
            "url": "https://hackertimes.com",
            "text": "<p>Built with Rust &amp; love.</p>",
            "score": 210,
            "descendants": 45
        });
        let thread = parse_hn_story_metadata(&val, 50346981);
        assert_eq!(thread.id, "50346981");
        assert_eq!(thread.subreddit.as_deref(), Some("hackernews"));
        assert_eq!(thread.title.as_deref(), Some("Show HN: Hacker Times"));
        assert_eq!(thread.author.as_deref(), Some("ht_author"));
        assert_eq!(thread.url.as_deref(), Some("https://hackertimes.com"));
        assert_eq!(thread.selftext.as_deref(), Some("Built with Rust & love."));
        assert_eq!(thread.score, 210);
        assert_eq!(thread.num_comments, 45);
    }
}
