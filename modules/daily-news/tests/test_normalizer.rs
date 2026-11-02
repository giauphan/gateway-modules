use daily_news::*;

struct DummyFetcher;
impl FeedFetcherPort for DummyFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(vec![])
    }
}

fn create_test_service(config: ModuleConfig) -> DailyNewsService {
    let fetcher = std::sync::Arc::new(DummyFetcher);
    let storage = std::sync::Arc::new(InMemoryStorageAdapter::with_config(config));
    DailyNewsService::new(fetcher, storage, CandidateScorer::default())
}

fn make_dummy_item(id: &str, url: &str, title: &str, pub_at: &str) -> RawNewsItem {
    RawNewsItem::new(id, url, title, pub_at)
}

#[test]
fn test_normalize_valid_item_and_storyboard_fields() {
    let svc = create_test_service(ModuleConfig::default());
    let mut raw = make_dummy_item(
        "item_123",
        "https://techcrunch.com/2026/11/02/ai-agents/?utm_source=feed",
        "Anthropic Announces MCP Improvements",
        "2026-11-02T10:00:00+00:00",
    );
    raw.publisher = Some("TechCrunch.com".to_string());
    raw.author = Some("Jane Doe".to_string());
    raw.description = Some("Anthropic published improvements to MCP.".to_string());
    raw.discussion_url = Some("https://news.ycombinator.com/item?id=50333339".to_string());
    raw.tags = vec!["ai".to_string(), "mcp".to_string()];
    raw.evidence.symptom = Some("Agent context bloat".to_string());
    raw.evidence.root_cause = Some("Eager tool registration".to_string());
    raw.evidence.remedy = Some("Two-tier schema scoping".to_string());
    raw.evidence.source_snapshot = Some("artifacts/s1.txt".to_string());
    raw.evidence.snapshot_sha256 = Some("2bad0815c06b65".to_string());

    let cand = svc.normalize_item(&raw).unwrap();
    assert!(cand.candidate_id.starts_with("cand_"));
    assert_eq!(
        cand.canonical_url,
        "https://techcrunch.com/2026/11/02/ai-agents"
    );
    assert_eq!(
        cand.source_url,
        "https://techcrunch.com/2026/11/02/ai-agents"
    );
    assert_eq!(cand.normalized_publisher, "TechCrunch");
    assert_eq!(
        cand.discussion_url.as_deref(),
        Some("https://news.ycombinator.com/item?id=50333339")
    );
    assert_eq!(
        cand.evidence.symptom.as_deref(),
        Some("Agent context bloat")
    );
    assert_eq!(
        cand.evidence.root_cause.as_deref(),
        Some("Eager tool registration")
    );
    assert_eq!(
        cand.evidence.remedy.as_deref(),
        Some("Two-tier schema scoping")
    );
    assert_eq!(
        cand.evidence.source_snapshot.as_deref(),
        Some("artifacts/s1.txt")
    );
    assert_eq!(
        cand.evidence.snapshot_sha256.as_deref(),
        Some("2bad0815c06b65")
    );

    // Verify serde flatten ensures flat JSON serialization for candidate contract
    let json_val = serde_json::to_value(&cand).unwrap();
    assert_eq!(json_val["symptom"], "Agent context bloat");
    assert_eq!(json_val["root_cause"], "Eager tool registration");
    assert_eq!(json_val["remedy"], "Two-tier schema scoping");
    assert_eq!(json_val["source_snapshot"], "artifacts/s1.txt");
    assert_eq!(json_val["snapshot_sha256"], "2bad0815c06b65");
}

#[test]
fn test_missing_required_fields_fails() {
    let svc = create_test_service(ModuleConfig::default());
    let mut raw_empty_id = make_dummy_item(
        "",
        "https://example.com",
        "Title",
        "2026-11-02T10:00:00+00:00",
    );
    raw_empty_id.id = "".to_string();
    assert!(svc.normalize_item(&raw_empty_id).is_err());
}

#[test]
fn test_batch_deduplication_and_date_filtering() {
    let cfg = ModuleConfig {
        freshness_window_hours: 48.0,
        require_latest_news: false,
        ..Default::default()
    };
    let svc = create_test_service(cfg);

    let raw1 = make_dummy_item(
        "1",
        "https://news.ycombinator.com/item?id=100",
        "New Rust Toolcase Gateway Released",
        "2026-11-02T11:00:00+00:00",
    );
    let raw2_duplicate = make_dummy_item(
        "2",
        "https://news.ycombinator.com/item?id=100&utm_source=twitter",
        "New Rust Toolcase Gateway Released",
        "2026-11-02T11:05:00+00:00",
    );
    let mut raw_evergreen = make_dummy_item(
        "eg_1",
        "https://git-scm.com/docs/git-worktree",
        "Git Worktree Documentation",
        "2015-07-28T00:00:00+00:00",
    );
    raw_evergreen.is_evergreen = true;
    raw_evergreen.updated_at = Some("2026-10-28T14:00:00+00:00".to_string());

    let items = vec![raw1, raw2_duplicate, raw_evergreen];
    let ranked = svc.filter_and_score(&items, None, None, None, 10);
    assert_eq!(ranked.len(), 2);
}

#[test]
fn test_require_latest_news_enforces_freshness_even_for_evergreen() {
    let cfg = ModuleConfig {
        freshness_window_hours: 48.0,
        require_latest_news: true,
        ..Default::default()
    };
    let svc = create_test_service(cfg);

    let mut stale_doc = make_dummy_item(
        "cand_old",
        "https://docs.anthropic.com/context-windows",
        "Why coding agents lose context",
        "2024-03-04T00:00:00+00:00",
    );
    stale_doc.is_evergreen = true;

    let ranked = svc.filter_and_score(&[stale_doc], None, None, None, 5);
    assert_eq!(ranked.len(), 0);
}
