//! Application service orchestrating news ingestion, ranking, and discovery use cases.
//!
//! Follows Clean Architecture Application Layer principles:
//! Ingests real-time live news feeds dynamically, enforces cross-source deduplication,
//! applies 6-axis ranking, and manages persistent consumption tracking.

use crate::domain::{
    canonicalize_url, classify_topic_family, compute_sha256, DiscoveryResult, ModuleConfig,
    NormalizedCandidate, RawNewsItem, RawProvenance,
};
use crate::ports::{FeedFetcherPort, StoragePort};
use crate::scoring::CandidateScorer;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

pub struct DailyNewsService {
    fetcher: Arc<dyn FeedFetcherPort>,
    storage: Arc<dyn StoragePort>,
    scorer: CandidateScorer,
}

impl DailyNewsService {
    pub fn new(
        fetcher: Arc<dyn FeedFetcherPort>,
        storage: Arc<dyn StoragePort>,
        scorer: CandidateScorer,
    ) -> Self {
        Self {
            fetcher,
            storage,
            scorer,
        }
    }

    pub fn normalize_item(&self, raw: &RawNewsItem) -> NormalizedCandidate {
        let canonical = canonicalize_url(&raw.url);
        let disc_url = raw
            .discussion_url
            .clone()
            .unwrap_or_else(|| raw.url.clone());
        let chash = compute_sha256(&format!("{}:{}", raw.title, canonical));
        let thash = compute_sha256(&raw.title);
        let topic = classify_topic_family(&raw.title, raw.description.as_deref().unwrap_or(""));
        let pub_name = raw
            .publisher
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        NormalizedCandidate {
            candidate_id: format!("cand_{}", &chash[..16]),
            canonical_url: canonical.clone(),
            original_url: raw.url.clone(),
            url: canonical,
            title: raw.title.clone(),
            publisher: pub_name.clone(),
            normalized_publisher: pub_name,
            published_at_utc: raw.published_at.clone(),
            published_at: raw.published_at.clone(),
            content_hash: chash,
            title_hash: thash,
            raw_provenance: RawProvenance {
                source_id: raw.id.clone(),
                source_feed: "live:hackernews".to_string(),
                extra: raw.extra.clone(),
            },
            adapter_version: "1.0.0".to_string(),
            topic_family: topic,
            author: raw.author.clone(),
            description: raw.description.clone(),
            body: raw.body.clone(),
            summary: raw.description.clone(),
            tags: raw.tags.clone(),
            language: "en".to_string(),
            image_url: None,
            is_evergreen: false,
            content_kind: "news".to_string(),
            updated_at: None,
            discussion_url: Some(disc_url),
            independent_fact_check: None,
            scoring: None,
        }
    }

    pub fn get_feed(&self, limit: usize) -> Vec<RawNewsItem> {
        self.fetcher.fetch_raw_items(limit).unwrap_or_default()
    }

    pub fn get_candidates(&self, limit: usize) -> Vec<NormalizedCandidate> {
        let raw = self.get_feed(limit.max(25));
        let candidates: Vec<NormalizedCandidate> =
            raw.iter().map(|r| self.normalize_item(r)).collect();
        let ranked = self.scorer.rank_candidates(candidates, limit);
        self.storage.cache_candidates(&ranked);
        ranked
    }

    /// Dynamic Live Content Discovery:
    /// Ingests live news items dynamically from upstream feeds, filters out consumed items,
    /// deduplicates by content & title hashes, matches topics, and returns ranked candidates.
    pub fn discover(
        &self,
        limit: usize,
        _from_date: Option<&str>,
        _to_date: Option<&str>,
        topics: Option<&[String]>,
    ) -> DiscoveryResult {
        let raw = self.get_feed(limit.max(30));
        let mut seen_content_hashes = HashSet::new();
        let mut seen_title_hashes = HashSet::new();
        let mut candidates = Vec::new();

        for r in &raw {
            let cand = self.normalize_item(r);
            if self.storage.is_consumed(&cand.candidate_id) {
                continue;
            }
            if !seen_content_hashes.insert(cand.content_hash.clone()) {
                continue;
            }
            if !seen_title_hashes.insert(cand.title_hash.clone()) {
                continue;
            }
            if let Some(topic_filter) = topics {
                if !topic_filter.iter().any(|t| t == &cand.topic_family) {
                    continue;
                }
            }
            candidates.push(cand);
        }

        let ranked = self.scorer.rank_candidates(candidates, limit);
        if !ranked.is_empty() {
            self.storage.cache_candidates(&ranked);
            let count = ranked.len();
            return DiscoveryResult {
                tier: "tier_2_automated".to_string(),
                status: "ELIGIBLE".to_string(),
                count,
                candidates: ranked,
                source_file: Some("live:hackernews".to_string()),
                reason: None,
                diagnostics: json!({
                    "tier_2_checked": true,
                    "engine": "rust-native",
                    "mode": "live-dynamic"
                }),
            };
        }

        // Safety Fallback when no fresh unconsumed items are available
        DiscoveryResult {
            tier: "tier_3_safety_fallback".to_string(),
            status: "FAIL_CLOSED".to_string(),
            count: 0,
            candidates: vec![],
            source_file: None,
            reason: Some(
                "No fresh candidates found matching topic or window criteria".to_string(),
            ),
            diagnostics: json!({
                "tier_2_checked": true,
                "tier_3_triggered": true,
                "engine": "rust-native"
            }),
        }
    }

    pub fn consume(&self, ids: &[String]) -> Result<usize, String> {
        self.storage.mark_consumed(ids)
    }

    pub fn get_config(&self) -> ModuleConfig {
        self.storage.load_config()
    }

    pub fn update_config(&self, new_config: ModuleConfig) -> Result<ModuleConfig, String> {
        self.storage.save_config(&new_config)?;
        Ok(new_config)
    }

    pub fn chat_completion(&self, limit: usize) -> String {
        let candidates = self.get_candidates(limit);
        let mut lines = vec![format!("### Daily News Briefing ({} items):\n", candidates.len())];
        for (i, c) in candidates.iter().enumerate() {
            let score = c.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
            let mut line = format!(
                "{}. **{}** ({}) [Score: {:.2}]\n   {}",
                i + 1,
                c.title,
                c.publisher,
                score,
                c.canonical_url
            );
            if let Some(ref d) = c.discussion_url {
                if d != &c.canonical_url {
                    line.push_str(&format!("\n   Discussion: {}", d));
                }
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    pub fn consumed_count(&self) -> usize {
        self.storage.consumed_count()
    }

    pub fn cached_candidates_count(&self) -> usize {
        self.storage.get_cached_candidates().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;

    struct MockFetcher;
    impl FeedFetcherPort for MockFetcher {
        fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
            Ok(vec![
                RawNewsItem {
                    id: "test_1".to_string(),
                    url: "https://example.com/item1?utm_medium=email".to_string(),
                    title: "First Live Article".to_string(),
                    published_at: "2026-11-02T10:00:00+00:00".to_string(),
                    publisher: Some("example.com".to_string()),
                    author: Some("johndoe".to_string()),
                    description: Some("Description here".to_string()),
                    body: None,
                    discussion_url: Some("https://news.ycombinator.com/item?id=123".to_string()),
                    tags: vec![],
                    extra: std::collections::HashMap::new(),
                },
                RawNewsItem {
                    id: "test_duplicate".to_string(),
                    url: "https://example.com/item1?utm_source=twitter".to_string(),
                    title: "First Live Article".to_string(),
                    published_at: "2026-11-02T10:05:00+00:00".to_string(),
                    publisher: Some("example.com".to_string()),
                    author: Some("johndoe".to_string()),
                    description: Some("Duplicate description".to_string()),
                    body: None,
                    discussion_url: Some("https://news.ycombinator.com/item?id=124".to_string()),
                    tags: vec![],
                    extra: std::collections::HashMap::new(),
                },
            ])
        }
    }

    struct MockStorage {
        consumed: RwLock<std::collections::HashSet<String>>,
    }
    impl StoragePort for MockStorage {
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
            ModuleConfig::default()
        }
        fn save_config(&self, _c: &ModuleConfig) -> Result<(), String> {
            Ok(())
        }
        fn cache_candidates(&self, _c: &[NormalizedCandidate]) {}
        fn get_cached_candidates(&self) -> Vec<NormalizedCandidate> {
            vec![]
        }
    }

    #[test]
    fn test_dynamic_discovery_and_deduplication() {
        let fetcher = Arc::new(MockFetcher);
        let storage = Arc::new(MockStorage {
            consumed: RwLock::new(std::collections::HashSet::new()),
        });
        let scorer = CandidateScorer::default();
        let service = DailyNewsService::new(fetcher, storage.clone(), scorer);

        // 1. Discover dynamically ingests live items and filters out duplicates by hash
        let res1 = service.discover(5, None, None, None);
        assert_eq!(res1.tier, "tier_2_automated");
        assert_eq!(res1.status, "ELIGIBLE");
        assert_eq!(res1.count, 1); // Duplicate item was filtered out
        assert_eq!(res1.candidates[0].canonical_url, "https://example.com/item1");
        assert_eq!(res1.candidates[0].url, "https://example.com/item1");
        assert_eq!(res1.candidates[0].publisher, "example.com");
        assert_eq!(res1.candidates[0].author.as_deref(), Some("johndoe"));

        // 2. Consume item
        let cand_id = &res1.candidates[0].candidate_id;
        service.consume(&[cand_id.clone()]).unwrap();

        // 3. Next discovery has no unconsumed items, safely falls back
        let res2 = service.discover(5, None, None, None);
        assert_eq!(res2.tier, "tier_3_safety_fallback");
        assert_eq!(res2.status, "FAIL_CLOSED");
        assert_eq!(res2.count, 0);
    }
}
