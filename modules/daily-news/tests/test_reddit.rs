use daily_news::*;
use serde_json::json;

#[test]
fn test_reddit_provider_initialization() {
    let p = RedditFeedProvider::new(vec![
        "LocalLLaMA".to_string(),
        "MachineLearning".to_string(),
    ])
    .with_credentials(
        Some("client_id_123".into()),
        Some("client_secret_456".into()),
    )
    .with_user_agent("TestAgent/1.0".into())
    .with_base_url(Some("http://127.0.0.1:8080".into()));

    assert_eq!(p.subreddits, vec!["LocalLLaMA", "MachineLearning"]);
    assert_eq!(p.client_id.as_deref(), Some("client_id_123"));
    assert_eq!(p.client_secret.as_deref(), Some("client_secret_456"));
    assert_eq!(p.user_agent, "TestAgent/1.0");
    assert_eq!(p.base_url.as_deref(), Some("http://127.0.0.1:8080"));
}

#[test]
fn test_parse_reddit_json_schema() {
    let raw = json!({
        "data": {
            "children": [
                {
                    "data": {
                        "id": "abc123",
                        "title": "DeepSeek-V3 Open Source Architecture Deep Dive",
                        "url": "https://arxiv.org/abs/2412.12345",
                        "permalink": "/r/LocalLLaMA/comments/abc123/deepseek_v3_open_source/",
                        "author": "ai_researcher",
                        "selftext": "Comprehensive analysis of MLA and deep thinking mechanisms.",
                        "created_utc": 1735000000,
                        "score": 1250,
                        "num_comments": 142,
                        "upvote_ratio": 0.98,
                        "is_self": false
                    }
                },
                {
                    "data": {
                        "id": "def456",
                        "title": "Ask LocalLLaMA: Best 8B model for code generation in 2026?",
                        "url": "https://www.reddit.com/r/LocalLLaMA/comments/def456/ask_localllama/",
                        "permalink": "/r/LocalLLaMA/comments/def456/ask_localllama/",
                        "author": "dev_user",
                        "selftext": "Evaluating Qwen2.5-Coder vs Llama-3.1-8B.",
                        "created_utc": 1735001000,
                        "score": 85,
                        "num_comments": 39,
                        "upvote_ratio": 0.91,
                        "is_self": true
                    }
                }
            ]
        }
    });

    let items = parse_reddit_json(&raw, "LocalLLaMA");
    assert_eq!(items.len(), 2);

    // 1. External link
    let item1 = &items[0];
    assert_eq!(item1.id, "reddit_LocalLLaMA_abc123");
    assert_eq!(
        item1.title,
        "DeepSeek-V3 Open Source Architecture Deep Dive"
    );
    assert_eq!(item1.url, "https://arxiv.org/abs/2412.12345");
    assert_eq!(item1.publisher.as_deref(), Some("arxiv.org"));
    assert_eq!(
        item1.discussion_url.as_deref(),
        Some("https://www.reddit.com/r/LocalLLaMA/comments/abc123/deepseek_v3_open_source/")
    );
    assert_eq!(item1.author.as_deref(), Some("ai_researcher"));
    assert_eq!(item1.extra["score"], 1250);
    assert_eq!(item1.extra["comments"], 142);
    assert_eq!(item1.tags, vec!["reddit", "localllama"]);

    // 2. Self post
    let item2 = &items[1];
    assert_eq!(item2.id, "reddit_LocalLLaMA_def456");
    assert_eq!(
        item2.url,
        "https://www.reddit.com/r/LocalLLaMA/comments/def456/ask_localllama/"
    );
    assert_eq!(item2.publisher.as_deref(), Some("reddit.com"));
    assert_eq!(
        item2.discussion_url.as_deref(),
        Some("https://www.reddit.com/r/LocalLLaMA/comments/def456/ask_localllama/")
    );
}

#[test]
fn test_arctic_shift_parsing_direct() {
    let raw = json!({
        "data": [{
            "id": "1xmlk2o",
            "title": "Qwen 3.5 Weights Edited by Claude",
            "url": "https://x.com/post/123",
            "permalink": "/r/LocalLLaMA/comments/1xmlk2o/qwen_post/",
            "author": "perelmanych",
            "selftext": "Full discussion of model weight edits.",
            "created_utc": 1793686979,
            "score": 1,
            "num_comments": 0,
            "is_self": false
        }]
    });

    let items = parse_arctic_json(&raw, "LocalLLaMA");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "reddit_LocalLLaMA_1xmlk2o");
    assert_eq!(items[0].title, "Qwen 3.5 Weights Edited by Claude");
    assert_eq!(items[0].url, "https://x.com/post/123");
    assert_eq!(items[0].publisher.as_deref(), Some("x.com"));
    assert_eq!(items[0].extra["provider"], "arctic_shift");
}

#[test]
fn test_fetcher_integration_with_reddit_offline() {
    let reddit = RedditFeedProvider::new(vec!["".to_string()])
        .with_base_url(Some("http://127.0.0.1:65534".to_string()));
    let fetcher = LiveFeedFetcher::new(None).with_reddit(reddit);
    let items = fetcher.fetch_sources(&["reddit".to_string()], 5).unwrap();
    assert!(items.is_empty());
}

#[test]
#[ignore = "requires live external internet access"]
fn test_live_arctic_call() {
    let items = fetch_arctic_subreddit("LocalLLaMA", 2, None);
    println!("test_live_arctic_call returned: {} items", items.len());
    for item in &items {
        println!(" - {} ({})", item.title, item.url);
    }
}
