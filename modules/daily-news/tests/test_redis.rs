use daily_news::*;

#[test]
fn test_redis_provider_addresses() {
    let p1 = RedisFeedProvider::new("redis://localhost:6379/0", "news:incoming", "stream");
    assert_eq!(p1.key, "news:incoming");
    assert_eq!(p1.channel_type, "stream");

    let p2 = RedisFeedProvider::new("redis://127.0.0.1:6380", "news:queue", "list");
    assert_eq!(p2.key, "news:queue");
    assert_eq!(p2.channel_type, "list");
}

#[test]
fn test_parse_raw_entry_formats() {
    // 1. JSON string with title, url, publisher
    let raw_json = r#"{
        "title": "Autonomous Coding Agent Benchmark Matrix",
        "url": "https://example.com/benchmark",
        "publisher": "AgentBench",
        "published_at": "2026-11-02T08:00:00+00:00",
        "discussion_url": "https://news.ycombinator.com/item?id=99999"
    }"#;
    let item1 = parse_raw_entry(raw_json, "msg_001", "news:queue", "list").unwrap();
    assert_eq!(item1.title, "Autonomous Coding Agent Benchmark Matrix");
    assert_eq!(item1.url, "https://example.com/benchmark");
    assert_eq!(item1.publisher.as_deref(), Some("AgentBench"));
    assert_eq!(
        item1.discussion_url.as_deref(),
        Some("https://news.ycombinator.com/item?id=99999")
    );
    assert_eq!(item1.content_origin, ContentOrigin::HeadlineOnly);
    assert_eq!(item1.published_at_kind, PublishedAtKind::ArticlePublication);

    // 2. Stream message ID timestamp fallback (<epoch_ms>-<seq>)
    // 1730541600000 ms = 1730541600 seconds (2024-11-02 10:00:00 UTC)
    let raw_no_pub = r#"{"title": "Stream Update Without Explicit Timestamp"}"#;
    let item2 = parse_raw_entry(raw_no_pub, "1730541600000-0", "news:stream", "stream").unwrap();
    assert_eq!(item2.title, "Stream Update Without Explicit Timestamp");
    assert!(item2.published_at.contains("2024-11-02"));
    assert_eq!(item2.url, "redis://news:stream/1730541600000-0");
    assert_eq!(item2.content_origin, ContentOrigin::HeadlineOnly);
    assert_eq!(item2.published_at_kind, PublishedAtKind::FeedObserved);

    // 3. Entry with explicit body and created_at
    let raw_body = r#"{
        "title": "Article with Real Body",
        "body": "Full body text.",
        "created_at": "2026-11-03T10:00:00Z"
    }"#;
    let item3 = parse_raw_entry(raw_body, "msg_003", "news:stream", "stream").unwrap();
    assert_eq!(item3.content_origin, ContentOrigin::Article);
    assert_eq!(item3.published_at_kind, PublishedAtKind::FeedObserved);
}

#[test]
fn test_offline_redis_resilience() {
    let provider = RedisFeedProvider::new("redis://127.0.0.1:65534", "news:incoming", "stream");
    let items = provider.fetch_items(5);
    assert!(items.is_empty());
}

#[test]
fn test_fetcher_integration_with_redis() {
    let redis = RedisFeedProvider::new("redis://127.0.0.1:65534", "news:incoming", "stream");
    let fetcher = LiveFeedFetcher::new(None).with_redis(redis);
    let items = fetcher.fetch_sources(&["redis".to_string()], 5).unwrap();
    assert!(items.is_empty());
}
