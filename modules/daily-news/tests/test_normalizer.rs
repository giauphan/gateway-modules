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

#[test]
fn test_normalize_valid_item_and_discussion_url() {
    let svc = create_test_service(ModuleConfig::default());
    let raw = RawNewsItem {
        id: "item_123".to_string(),
        url: "https://techcrunch.com/2026/11/02/ai-agents/?utm_source=feed".to_string(),
        title: "Anthropic Announces MCP Improvements".to_string(),
        published_at: "2026-11-02T10:00:00+00:00".to_string(),
        publisher: Some("TechCrunch.com".to_string()),
        author: Some("Jane Doe".to_string()),
        description: Some("Anthropic published improvements to MCP.".to_string()),
        body: None,
        discussion_url: Some("https://news.ycombinator.com/item?id=50333339".to_string()),
        tags: vec!["ai".to_string(), "mcp".to_string()],
        is_evergreen: false,
        content_kind: None,
        updated_at: None,
        extra: std::collections::HashMap::new(),
    };

    let cand = svc.normalize_item(&raw).unwrap();
    assert!(cand.candidate_id.starts_with("cand_"));
    assert_eq!(cand.canonical_url, "https://techcrunch.com/2026/11/02/ai-agents");
    assert_eq!(cand.normalized_publisher, "TechCrunch");
    assert_eq!(cand.discussion_url.as_deref(), Some("https://news.ycombinator.com/item?id=50333339"));
}

#[test]
fn test_missing_required_fields_fails() {
    let svc = create_test_service(ModuleConfig::default());
    let raw_empty_id = RawNewsItem {
        id: "".to_string(),
        url: "https://example.com".to_string(),
        title: "Title".to_string(),
        published_at: "2026-11-02T10:00:00+00:00".to_string(),
        publisher: None,
        author: None,
        description: None,
        body: None,
        discussion_url: None,
        tags: vec![],
        is_evergreen: false,
        content_kind: None,
        updated_at: None,
        extra: std::collections::HashMap::new(),
    };
    assert!(svc.normalize_item(&raw_empty_id).is_err());
}

#[test]
fn test_batch_deduplication_and_date_filtering() {
    let mut cfg = ModuleConfig::default();
    cfg.freshness_window_hours = 48.0;
    cfg.require_latest_news = false;
    let svc = create_test_service(cfg);

    let raw1 = RawNewsItem {
        id: "1".to_string(),
        url: "https://news.ycombinator.com/item?id=100".to_string(),
        title: "New Rust Toolcase Gateway Released".to_string(),
        published_at: "2026-11-02T11:00:00+00:00".to_string(),
        publisher: None,
        author: None,
        description: None,
        body: None,
        discussion_url: None,
        tags: vec![],
        is_evergreen: false,
        content_kind: None,
        updated_at: None,
        extra: std::collections::HashMap::new(),
    };
    let raw2_duplicate = RawNewsItem {
        id: "2".to_string(),
        url: "https://news.ycombinator.com/item?id=100&utm_source=twitter".to_string(),
        title: "New Rust Toolcase Gateway Released".to_string(),
        published_at: "2026-11-02T11:05:00+00:00".to_string(),
        publisher: None,
        author: None,
        description: None,
        body: None,
        discussion_url: None,
        tags: vec![],
        is_evergreen: false,
        content_kind: None,
        updated_at: None,
        extra: std::collections::HashMap::new(),
    };
    let raw_evergreen = RawNewsItem {
        id: "eg_1".to_string(),
        url: "https://git-scm.com/docs/git-worktree".to_string(),
        title: "Git Worktree Documentation".to_string(),
        published_at: "2015-07-28T00:00:00+00:00".to_string(),
        publisher: Some("git-scm.com".to_string()),
        author: None,
        description: None,
        body: None,
        discussion_url: None,
        tags: vec![],
        is_evergreen: true,
        content_kind: Some("documentation".to_string()),
        updated_at: Some("2026-10-28T14:00:00+00:00".to_string()),
        extra: std::collections::HashMap::new(),
    };

    let items = vec![raw1, raw2_duplicate, raw_evergreen];
    let ranked = svc.filter_and_score(&items, None, None, None, 10);

    // Duplicate was filtered out, evergreen is preserved because require_latest_news is false
    assert_eq!(ranked.len(), 2);
}

#[test]
fn test_require_latest_news_enforces_freshness_even_for_evergreen() {
    let mut cfg = ModuleConfig::default();
    cfg.freshness_window_hours = 48.0;
    cfg.require_latest_news = true;
    let svc = create_test_service(cfg);

    let stale_doc = RawNewsItem {
        id: "cand_old".to_string(),
        url: "https://docs.anthropic.com/context-windows".to_string(),
        title: "Why coding agents lose context".to_string(),
        published_at: "2024-03-04T00:00:00+00:00".to_string(),
        publisher: None,
        author: None,
        description: None,
        body: None,
        discussion_url: None,
        tags: vec![],
        is_evergreen: true,
        content_kind: None,
        updated_at: None,
        extra: std::collections::HashMap::new(),
    };

    let ranked = svc.filter_and_score(&[stale_doc], None, None, None, 5);
    assert_eq!(ranked.len(), 0);
}
