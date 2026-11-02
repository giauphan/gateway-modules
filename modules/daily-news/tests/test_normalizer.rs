use daily_news::*;
use std::collections::HashSet;
use std::sync::RwLock;

struct DummyFetcher;
impl FeedFetcherPort for DummyFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(vec![])
    }
}

struct InMemoryStorage {
    config: RwLock<ModuleConfig>,
    consumed: RwLock<HashSet<String>>,
    cached: RwLock<Vec<NormalizedCandidate>>,
}

impl StoragePort for InMemoryStorage {
    fn is_consumed(&self, id: &str) -> bool {
        self.consumed.read().unwrap().contains(id)
    }
    fn mark_consumed(&self, ids: &[String]) -> Result<usize, String> {
        let mut count = 0;
        let mut guard = self.consumed.write().unwrap();
        for id in ids {
            if guard.insert(id.clone()) {
                count += 1;
            }
        }
        Ok(count)
    }
    fn consumed_count(&self) -> usize {
        self.consumed.read().unwrap().len()
    }
    fn load_config(&self) -> ModuleConfig {
        self.config.read().unwrap().clone()
    }
    fn save_config(&self, c: &ModuleConfig) -> Result<(), String> {
        *self.config.write().unwrap() = c.clone();
        Ok(())
    }
    fn cache_candidates(&self, c: &[NormalizedCandidate]) {
        *self.cached.write().unwrap() = c.to_vec();
    }
    fn get_cached_candidates(&self) -> Vec<NormalizedCandidate> {
        self.cached.read().unwrap().clone()
    }
}

fn create_test_service(config: ModuleConfig) -> DailyNewsService {
    let fetcher = std::sync::Arc::new(DummyFetcher);
    let storage = std::sync::Arc::new(InMemoryStorage {
        config: RwLock::new(config),
        consumed: RwLock::new(HashSet::new()),
        cached: RwLock::new(Vec::new()),
    });
    DailyNewsService::new(fetcher, storage, CandidateScorer::default())
}

fn make_dummy_item(id: &str, url: &str, title: &str, pub_at: &str) -> RawNewsItem {
    RawNewsItem {
        id: id.to_string(),
        url: url.to_string(),
        title: title.to_string(),
        published_at: pub_at.to_string(),
        publisher: None,
        author: None,
        description: None,
        body: None,
        discussion_url: None,
        tags: vec![],
        is_evergreen: false,
        content_kind: None,
        updated_at: None,
        hook: None,
        viewer_question: None,
        symptom: None,
        root_cause: None,
        remedy: None,
        evidence_excerpt: None,
        source_snapshot: None,
        snapshot_sha256: None,
        extra: std::collections::HashMap::new(),
    }
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
    raw.symptom = Some("Agent context bloat".to_string());
    raw.root_cause = Some("Eager tool registration".to_string());
    raw.remedy = Some("Two-tier schema scoping".to_string());
    raw.source_snapshot = Some("artifacts/s1.txt".to_string());
    raw.snapshot_sha256 = Some("2bad0815c06b65".to_string());

    let cand = svc.normalize_item(&raw).unwrap();
    assert!(cand.candidate_id.starts_with("cand_"));
    assert_eq!(cand.canonical_url, "https://techcrunch.com/2026/11/02/ai-agents");
    assert_eq!(cand.source_url, "https://techcrunch.com/2026/11/02/ai-agents");
    assert_eq!(cand.normalized_publisher, "TechCrunch");
    assert_eq!(cand.discussion_url.as_deref(), Some("https://news.ycombinator.com/item?id=50333339"));
    assert_eq!(cand.symptom.as_deref(), Some("Agent context bloat"));
    assert_eq!(cand.root_cause.as_deref(), Some("Eager tool registration"));
    assert_eq!(cand.remedy.as_deref(), Some("Two-tier schema scoping"));
    assert_eq!(cand.source_snapshot.as_deref(), Some("artifacts/s1.txt"));
    assert_eq!(cand.snapshot_sha256.as_deref(), Some("2bad0815c06b65"));
}

#[test]
fn test_missing_required_fields_fails() {
    let svc = create_test_service(ModuleConfig::default());
    let mut raw_empty_id = make_dummy_item("", "https://example.com", "Title", "2026-11-02T10:00:00+00:00");
    raw_empty_id.id = "".to_string();
    assert!(svc.normalize_item(&raw_empty_id).is_err());
}

#[test]
fn test_batch_deduplication_and_date_filtering() {
    let mut cfg = ModuleConfig::default();
    cfg.freshness_window_hours = 48.0;
    cfg.require_latest_news = false;
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
    let mut cfg = ModuleConfig::default();
    cfg.freshness_window_hours = 48.0;
    cfg.require_latest_news = true;
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
