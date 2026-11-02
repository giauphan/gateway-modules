use daily_news::*;
use std::collections::HashSet;
use std::sync::RwLock;

struct MockDiscoveryFetcher {
    items: Vec<RawNewsItem>,
}

impl FeedFetcherPort for MockDiscoveryFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(self.items.clone())
    }
    fn fetch_sources(&self, _sources: &[String], _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        Ok(self.items.clone())
    }
}

struct MockDiscoveryStorage {
    consumed: RwLock<HashSet<String>>,
    config: RwLock<ModuleConfig>,
    cached: RwLock<Vec<NormalizedCandidate>>,
}

impl StoragePort for MockDiscoveryStorage {
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

#[test]
fn test_tier_1_curated_priority_and_consumed() {
    let storage = std::sync::Arc::new(MockDiscoveryStorage {
        consumed: RwLock::new(HashSet::new()),
        config: RwLock::new(ModuleConfig::default()),
        cached: RwLock::new(Vec::new()),
    });
    let fetcher = std::sync::Arc::new(MockDiscoveryFetcher { items: vec![] });
    let service = DailyNewsService::new(fetcher, storage.clone(), CandidateScorer::default());

    let mut curated_item = RawNewsItem::new(
        "c1",
        "https://docs.anthropic.com/model-context-protocol",
        "Model Context Protocol Specification 1.0",
        "2026-11-02T10:00:00+00:00",
    );
    curated_item.publisher = Some("Anthropic".to_string());
    curated_item.description = Some("Official specification for tool schemas".to_string());

    let curated = vec![curated_item];

    // 1. Initial discovery matches Tier 1
    let res = service.discover(5, Some(curated.clone()), None, None, None, None).unwrap();
    assert_eq!(res.tier, "tier_1_curated");
    assert_eq!(res.status, "ELIGIBLE");
    assert_eq!(res.count, 1);
    let cid = res.candidates[0].candidate_id.clone();

    // 2. Consume item
    service.consume(&[cid]).unwrap();

    // 3. Repeat discovery fails closed
    let res2 = service.discover(5, Some(curated), None, None, None, None).unwrap();
    assert_eq!(res2.tier, "none");
    assert_eq!(res2.status, "NO_ELIGIBLE_TOPIC");
    assert_eq!(res2.count, 0);
}

#[test]
fn test_topic_filtering_and_fail_closed() {
    let mut raw_tool = RawNewsItem::new(
        "t1",
        "https://mcp.io/spec",
        "Model Context Protocol Tool Schema Update",
        "2026-11-02T10:00:00+00:00",
    );
    raw_tool.description = Some("MCP function calls and tool schemas".to_string());

    let mut raw_debug = RawNewsItem::new(
        "t2",
        "https://ci.dev/flaky",
        "Debugging Flaky Test Retries in CI",
        "2026-11-02T10:00:00+00:00",
    );
    raw_debug.description = Some("Flaky CI loop debug and crash traces".to_string());

    let fetcher = std::sync::Arc::new(MockDiscoveryFetcher {
        items: vec![raw_tool, raw_debug],
    });
    let storage = std::sync::Arc::new(MockDiscoveryStorage {
        consumed: RwLock::new(HashSet::new()),
        config: RwLock::new(ModuleConfig::default()),
        cached: RwLock::new(Vec::new()),
    });
    let service = DailyNewsService::new(fetcher, storage, CandidateScorer::default());

    // 1. Filter by agent_tool_use
    let res_tool = service.discover(5, None, None, None, None, Some(&["agent_tool_use".to_string()])).unwrap();
    assert_eq!(res_tool.status, "ELIGIBLE");
    assert_eq!(res_tool.candidates.len(), 1);
    assert_eq!(res_tool.candidates[0].topic_family, "agent_tool_use");

    // 2. Filter by multiple topics
    let topics = vec!["agent_tool_use".to_string(), "debugging_reliability".to_string()];
    let res_multi = service.discover(5, None, None, None, None, Some(&topics)).unwrap();
    assert_eq!(res_multi.status, "ELIGIBLE");
    assert_eq!(res_multi.candidates.len(), 2);

    // 3. Filter by non-existent topic fails closed
    let res_none = service.discover(5, None, None, None, None, Some(&["crypto_unknown".to_string()])).unwrap();
    assert_eq!(res_none.status, "NO_ELIGIBLE_TOPIC");
    assert_eq!(res_none.candidates.len(), 0);
}

#[test]
fn test_date_range_filtering_and_validation() {
    let d1 = RawNewsItem::new(
        "d1",
        "https://news.ycombinator.com/item?id=101",
        "Rust 2026 Edition Announced",
        "2026-10-29T10:00:00+00:00",
    );
    let d2 = RawNewsItem::new(
        "d2",
        "https://news.ycombinator.com/item?id=102",
        "DeepMind Launches New Agent",
        "2026-10-31T15:00:00+00:00",
    );

    let fetcher = std::sync::Arc::new(MockDiscoveryFetcher { items: vec![d1, d2] });
    let storage = std::sync::Arc::new(MockDiscoveryStorage {
        consumed: RwLock::new(HashSet::new()),
        config: RwLock::new(ModuleConfig::default()),
        cached: RwLock::new(Vec::new()),
    });
    let service = DailyNewsService::new(fetcher, storage, CandidateScorer::default());

    // 1. Search Oct 30 to Nov 01 -> matches only d2
    let res = service.discover(5, None, None, Some("2026-10-30"), Some("2026-11-01"), None).unwrap();
    assert_eq!(res.status, "ELIGIBLE");
    assert_eq!(res.candidates.len(), 1);
    assert_eq!(res.candidates[0].title, "DeepMind Launches New Agent");

    // 2. Inverted date range returns error
    let err_inv = service.discover(5, None, None, Some("2026-11-05"), Some("2026-11-01"), None);
    assert!(err_inv.is_err());
    assert_eq!(err_inv.unwrap_err().0, "INVALID_DATE_RANGE");

    // 3. Invalid date format returns error
    let err_fmt = service.discover(5, None, None, Some("not-a-date"), None, None);
    assert!(err_fmt.is_err());
    assert_eq!(err_fmt.unwrap_err().0, "INVALID_DATE_FORMAT");
}
