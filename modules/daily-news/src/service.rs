//! Application service orchestrating news ingestion, ranking, and discovery use cases.
//!
//! Follows Clean Architecture Application Layer principles:
//! Implements 3-tier cascade discovery (Tier 1 Curated -> Tier 2 Automated -> Tier 3 Safety Fallback),
//! cross-source deduplication, and persistent candidate consumption.

use crate::domain::{
    canonicalize_url, classify_topic_family, compute_sha256, DiscoveryResult, ModuleConfig,
    NormalizedCandidate, RawNewsItem,
};
use crate::ports::{FeedFetcherPort, StoragePort};
use crate::scoring::CandidateScorer;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

const OFFICIAL_SOURCES_RAW: &str = include_str!("../data/official_sources.json");

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

        NormalizedCandidate {
            candidate_id: format!("cand_{}", &chash[..16]),
            canonical_url: canonical,
            original_url: raw.url.clone(),
            title: raw.title.clone(),
            publisher: raw.publisher.clone().unwrap_or_else(|| "Unknown".to_string()),
            normalized_publisher: raw.publisher.clone().unwrap_or_else(|| "Unknown".to_string()),
            published_at_utc: raw.published_at.clone(),
            content_hash: chash,
            title_hash: thash,
            description: raw.description.clone(),
            body: raw.body.clone(),
            summary: raw.description.clone(),
            discussion_url: Some(disc_url),
            topic_family: topic,
            tags: raw.tags.clone(),
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

    /// Tiered Cascading Discovery:
    /// - Tier 1: Curated Queue from official sources (verified documentation & standards).
    /// - Tier 2: Automated Feed from Hacker News / NewsNow with deduplication.
    /// - Tier 3: Fail-Closed Safety Fallback when no fresh eligible items match criteria.
    pub fn discover(
        &self,
        limit: usize,
        _from_date: Option<&str>,
        _to_date: Option<&str>,
        topics: Option<&[String]>,
    ) -> DiscoveryResult {
        // --- 1. Tier 1: Curated Queue ---
        let curated_raw: Vec<serde_json::Value> =
            serde_json::from_str(OFFICIAL_SOURCES_RAW).unwrap_or_default();
        let mut tier_1_candidates = Vec::new();
        for val in curated_raw {
            let cid = val["candidate_id"]
                .as_str()
                .or_else(|| val["id"].as_str())
                .unwrap_or("")
                .to_string();
            if cid.is_empty() || self.storage.is_consumed(&cid) {
                continue;
            }
            let topic = val["topic_family"]
                .as_str()
                .unwrap_or("agent_architecture")
                .to_string();
            if let Some(topic_filter) = topics {
                if !topic_filter.iter().any(|t| t == &topic) {
                    continue;
                }
            }
            let title = val["title"].as_str().unwrap_or("").to_string();
            let canonical = val["canonical_url"]
                .as_str()
                .or_else(|| val["url"].as_str())
                .unwrap_or("")
                .to_string();
            let pub_name = val["publisher"].as_str().unwrap_or("Official").to_string();
            let pub_utc = val["published_at_utc"]
                .as_str()
                .or_else(|| val["published_at"].as_str())
                .unwrap_or("2026-11-02T11:00:00Z")
                .to_string();
            let body = val["body"].as_str().map(str::to_string);
            let chash = compute_sha256(&format!("{title}:{canonical}"));
            let thash = compute_sha256(&title);

            let mut cand = NormalizedCandidate {
                candidate_id: cid,
                canonical_url: canonical.clone(),
                original_url: canonical,
                title,
                publisher: pub_name.clone(),
                normalized_publisher: pub_name,
                published_at_utc: pub_utc,
                content_hash: chash,
                title_hash: thash,
                description: body.clone(),
                body,
                summary: None,
                discussion_url: None,
                topic_family: topic,
                tags: vec!["official".to_string(), "documentation".to_string()],
                scoring: None,
            };
            cand.scoring = Some(self.scorer.score_candidate(&cand));
            tier_1_candidates.push(cand);
        }

        if !tier_1_candidates.is_empty() {
            tier_1_candidates.sort_by(|a, b| {
                let sa = a.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
                let sb = b.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
            tier_1_candidates.truncate(limit);
            let count = tier_1_candidates.len();
            self.storage.cache_candidates(&tier_1_candidates);
            return DiscoveryResult {
                tier: "tier_1_curated".to_string(),
                status: "ELIGIBLE".to_string(),
                count,
                candidates: tier_1_candidates,
                source_file: Some("data/official_sources.json".to_string()),
                reason: None,
                diagnostics: json!({
                    "tier_1_checked": true,
                    "engine": "rust-native"
                }),
            };
        }

        // --- 2. Tier 2: Automated Feed ---
        let raw = self.get_feed(limit.max(30));
        let mut seen_content_hashes = HashSet::new();
        let mut seen_title_hashes = HashSet::new();
        let mut tier_2_candidates = Vec::new();

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
            tier_2_candidates.push(cand);
        }

        let ranked = self.scorer.rank_candidates(tier_2_candidates, limit);
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
                    "tier_1_checked": true,
                    "tier_2_checked": true,
                    "engine": "rust-native"
                }),
            };
        }

        // --- 3. Tier 3: Safety Fallback ---
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
                "tier_1_checked": true,
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
            Ok(vec![RawNewsItem {
                id: "test_1".to_string(),
                url: "https://example.com/item1?utm_medium=email".to_string(),
                title: "Test Article".to_string(),
                published_at: "2026-11-02T10:00:00Z".to_string(),
                publisher: Some("TestLab".to_string()),
                description: Some("Description here".to_string()),
                body: None,
                discussion_url: Some("https://news.ycombinator.com/item?id=123".to_string()),
                tags: vec![],
                extra: std::collections::HashMap::new(),
            }])
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
    fn test_tiered_discovery_cascade() {
        let fetcher = Arc::new(MockFetcher);
        let storage = Arc::new(MockStorage {
            consumed: RwLock::new(std::collections::HashSet::new()),
        });
        let scorer = CandidateScorer::default();
        let service = DailyNewsService::new(fetcher, storage.clone(), scorer);

        // 1. Initial discover returns Tier 1 curated items
        let res1 = service.discover(2, None, None, Some(&["context_memory".to_string()]));
        assert_eq!(res1.tier, "tier_1_curated");
        assert_eq!(res1.status, "ELIGIBLE");
        assert!(res1.count > 0);

        // 2. Consume all items in official_sources.json
        let all_curated: Vec<serde_json::Value> =
            serde_json::from_str(OFFICIAL_SOURCES_RAW).unwrap_or_default();
        let all_ids: Vec<String> = all_curated
            .iter()
            .filter_map(|v| {
                v["candidate_id"]
                    .as_str()
                    .or_else(|| v["id"].as_str())
                    .map(str::to_string)
            })
            .collect();
        service.consume(&all_ids).unwrap();

        // 3. Next discover has no curated items left, so it cascades to Tier 2 automated
        let res2 = service.discover(2, None, None, None);
        assert_eq!(res2.tier, "tier_2_automated");
        assert_eq!(res2.status, "ELIGIBLE");
        assert_eq!(res2.candidates[0].canonical_url, "https://example.com/item1");
    }
}
