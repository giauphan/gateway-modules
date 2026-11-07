//! Application service orchestrating news ingestion, ranking, and tiered discovery use cases.
//!
//! Follows Clean Architecture Application Layer principles:
//! Ingests real-time live news feeds, enforces cross-source deduplication,
//! applies 6-axis ranking, and manages persistent consumption tracking.

use crate::discovery::{
    build_fail_closed_result, validate_date_bounds,
};
use crate::domain::{
    DiscoveryResult, DiscussionThread, ModuleConfig, NormalizedCandidate, RawNewsItem,
};
use crate::ports::{
    DiscussionFetcherPort, FeedFetcherPort, NoopDiscussionFetcher, SourceCapturePort, StoragePort,
};
use crate::scoring::CandidateScorer;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

mod handoff;
mod integrity;

pub struct DailyNewsService {
    fetcher: std::sync::RwLock<Arc<dyn FeedFetcherPort>>,
    storage: Arc<dyn StoragePort>,
    scorer: CandidateScorer,
    discussion: Arc<dyn DiscussionFetcherPort>,
    capture: Option<Arc<dyn SourceCapturePort>>,
}

impl DailyNewsService {
    pub fn new(
        fetcher: Arc<dyn FeedFetcherPort>,
        storage: Arc<dyn StoragePort>,
        scorer: CandidateScorer,
    ) -> Self {
        Self {
            fetcher: std::sync::RwLock::new(fetcher),
            storage,
            scorer,
            discussion: Arc::new(NoopDiscussionFetcher),
            capture: None,
        }
    }

    fn fetcher(&self) -> Arc<dyn FeedFetcherPort> {
        self.fetcher
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn with_discussion_fetcher(mut self, discussion: Arc<dyn DiscussionFetcherPort>) -> Self {
        self.discussion = discussion;
        self
    }

    pub fn with_source_capture(mut self, capture: Arc<dyn SourceCapturePort>) -> Self {
        self.capture = Some(capture);
        self
    }

    pub fn get_discussion(
        &self,
        url_or_id: &str,
        limit: Option<usize>,
    ) -> Result<DiscussionThread, String> {
        self.discussion.fetch_thread(url_or_id, limit)
    }

    pub fn capture_primary_source(
        &self,
        url: &str,
    ) -> Result<crate::domain::PrimarySourceEvidence, String> {
        match self.capture {
            Some(ref c) => c.capture_source(url),
            None => Err("Source capture adapter is not configured".to_string()),
        }
    }

    pub fn enrich_candidate(
        &self,
        candidate_id: &str,
        override_url: Option<&str>,
        claims: Option<Vec<crate::domain::ClaimEvidence>>,
        visuals: Option<Vec<crate::domain::VisualEvidence>>,
    ) -> Result<crate::enrichment::EnrichedCandidateResponse, String> {
        let workflow = crate::enrichment::CandidateEnrichmentWorkflow::new(
            self.storage.clone(),
            self.capture.clone(),
            CandidateScorer::default(),
        );
        workflow.enrich_candidate(candidate_id, override_url, claims, visuals)
    }

    pub fn normalize_item(&self, raw: &RawNewsItem) -> Result<NormalizedCandidate, String> {
        let snapshot_text = raw
            .primary_source
            .as_ref()
            .and_then(|ps| ps.snapshot_ref.as_deref())
            .and_then(|ref_str| {
                self.capture
                    .as_ref()
                    .and_then(|cap| cap.read_snapshot_text(ref_str))
            });

        let secondary_reader = |hash: &str| -> Option<String> {
            self.capture
                .as_ref()
                .and_then(|cap| cap.read_snapshot_text(hash))
        };

        let audit_reader = |hash: &str| -> Option<String> {
            self.capture
                .as_ref()
                .and_then(|cap| cap.read_snapshot_text(hash))
        };

        let asset_reader = |hash: &str| -> Option<Vec<u8>> {
            self.capture
                .as_ref()
                .and_then(|cap| cap.read_snapshot_bytes(hash))
        };

        crate::discovery::normalize_raw_item_full(
            raw,
            snapshot_text.as_deref(),
            Some(&secondary_reader),
            Some(&audit_reader),
            Some(&asset_reader),
        )
    }

    pub fn filter_and_score(
        &self,
        raw_items: &[RawNewsItem],
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        topics: Option<&[String]>,
        limit: usize,
    ) -> Vec<NormalizedCandidate> {
        let cfg = self.storage.load_config();
        let policy = crate::discovery::CandidateEligibilityPolicy::from_config(
            &cfg, from_epoch, to_epoch, topics,
        );

        let mut seen_content_hashes = HashSet::new();
        let mut seen_title_hashes = HashSet::new();
        let mut unconsumed = Vec::new();

        for r in raw_items {
            let cand = match self.normalize_item(r) {
                Ok(c) => c,
                Err(_) => continue,
            };

            match self.storage.try_is_consumed(&cand.candidate_id) {
                Ok(true) => continue,
                Err(e) => {
                    eprintln!("[daily-news] Storage error checking consumed status for candidate '{}': {e}", cand.candidate_id);
                    continue;
                }
                Ok(false) => {}
            }

            if !policy.is_candidate_eligible(
                &cand,
                &mut seen_content_hashes,
                &mut seen_title_hashes,
            ) {
                continue;
            }

            unconsumed.push(cand);
        }

        self.scorer.rank_candidates(unconsumed, limit)
    }

    fn persist_and_build_discovery_result(
        &self,
        candidates: Vec<NormalizedCandidate>,
        tier: &str,
        source_file: &str,
    ) -> Result<Option<DiscoveryResult>, (String, String)> {
        if candidates.is_empty() {
            return Ok(None);
        }
        self.storage
            .save_candidates(&candidates)
            .map_err(|e| ("PERSISTENCE_ERROR".to_string(), format!("Failed to persist candidates: {e}")))?;
        let count = candidates.len();
        Ok(Some(DiscoveryResult {
            tier: tier.to_string(),
            status: "ELIGIBLE".to_string(),
            count,
            candidates,
            source_file: Some(source_file.to_string()),
            reason: None,
            diagnostics: json!({ "tier": tier }),
        }))
    }

    fn try_curated_tier(
        &self,
        curated: Option<Vec<RawNewsItem>>,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        topics: Option<&[String]>,
        count: usize,
    ) -> Result<Option<DiscoveryResult>, (String, String)> {
        let feed = match curated {
            Some(f) => f,
            None => return Ok(None),
        };
        if feed.is_empty() {
            return Ok(None);
        }
        let ranked = self.filter_and_score(&feed, from_epoch, to_epoch, topics, count);
        self.persist_and_build_discovery_result(ranked, "tier_1_curated", "curated_feed")
    }

    fn try_live_tier(
        &self,
        sources: Option<&[String]>,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        topics: Option<&[String]>,
        count: usize,
    ) -> Result<Option<DiscoveryResult>, (String, String)> {
        if let Some(s) = sources {
            if s.is_empty() {
                return Ok(None);
            }
        }
        let cfg = self.storage.load_config();
        let effective_sources: Vec<String> = sources
            .map(<[String]>::to_vec)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cfg.default_sources.clone());

        let raw = self
            .fetcher()
            .fetch_sources(&effective_sources, count.max(30))
            .unwrap_or_default();
        let ranked = self.filter_and_score(&raw, from_epoch, to_epoch, topics, count);
        self.persist_and_build_discovery_result(ranked, "tier_2_automated", "live_feeds")
    }

    fn try_cache_tier(
        &self,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        topics: Option<&[String]>,
        count: usize,
    ) -> Option<DiscoveryResult> {
        let cfg = self.storage.load_config();
        let policy = crate::discovery::CandidateEligibilityPolicy::from_config(
            &cfg,
            from_epoch,
            to_epoch,
            topics,
        );

        let cached = self.storage.get_cached_candidates();
        let mut seen_content_hashes = HashSet::new();
        let mut seen_title_hashes = HashSet::new();

        let eligible: Vec<NormalizedCandidate> = cached
            .into_iter()
            .filter(|c| match self.storage.try_is_consumed(&c.candidate_id) {
                Ok(false) => true,
                Ok(true) => false,
                Err(e) => {
                    eprintln!("[daily-news] Storage error checking consumed status for cached candidate '{}': {e}", c.candidate_id);
                    false
                }
            })
            .filter(|c| policy.is_candidate_eligible(c, &mut seen_content_hashes, &mut seen_title_hashes))
            .collect();

        if eligible.is_empty() {
            return None;
        }

        let ranked = self.scorer.rank_candidates(eligible, count);
        let c_len = ranked.len();
        Some(DiscoveryResult {
            tier: "tier_2_cache".to_string(),
            status: "ELIGIBLE".to_string(),
            count: c_len,
            candidates: ranked,
            source_file: Some("cached_candidates".to_string()),
            reason: None,
            diagnostics: json!({ "tier": "tier_2_cache" }),
        })
    }

    pub fn discover(
        &self,
        count: usize,
        curated: Option<Vec<RawNewsItem>>,
        sources: Option<Vec<String>>,
        from_date_str: Option<&str>,
        to_date_str: Option<&str>,
        topics: Option<&[String]>,
    ) -> Result<DiscoveryResult, (String, String)> {
        let (from_epoch, to_epoch) = validate_date_bounds(from_date_str, to_date_str)?;

        if let Some(res) = self.try_curated_tier(curated, from_epoch, to_epoch, topics, count)? {
            return Ok(res);
        }
        if let Some(res) =
            self.try_live_tier(sources.as_deref(), from_epoch, to_epoch, topics, count)?
        {
            return Ok(res);
        }
        if let Some(res) = self.try_cache_tier(from_epoch, to_epoch, topics, count) {
            return Ok(res);
        }

        Ok(build_fail_closed_result(topics, from_date_str, to_date_str))
    }

    pub fn get_feed(&self, limit: usize) -> Vec<RawNewsItem> {
        let mut items = self.fetcher().fetch_raw_items(limit).unwrap_or_default();
        items.truncate(limit);
        items
    }

    pub fn get_feed_sources(&self, sources: &[String], limit: usize) -> Vec<RawNewsItem> {
        let mut items = self
            .fetcher()
            .fetch_sources(sources, limit)
            .unwrap_or_default();
        items.truncate(limit);
        items
    }

    pub fn try_get_candidates(&self, limit: usize) -> Result<Vec<NormalizedCandidate>, String> {
        let cfg = self.storage.load_config();
        let policy = crate::discovery::CandidateEligibilityPolicy::from_config(&cfg, None, None, None);
        let mut all_candidates = self.storage.get_cached_candidates();
        let raw = self.get_feed(limit.max(25));
        let mut updated = false;

        for r in raw {
            if let Ok(c) = self.normalize_item(&r) {
                if let Some(existing) = all_candidates
                    .iter_mut()
                    .find(|existing| existing.candidate_id == c.candidate_id)
                {
                    crate::storage::merge_candidate_entry(existing, &c);
                    updated = true;
                } else {
                    all_candidates.push(c);
                    updated = true;
                }
            }
        }

        // Reverify cached readiness in place on all_candidates before filtering/ranking
        for c in &mut all_candidates {
            let is_consumed = match self.storage.try_is_consumed(&c.candidate_id) {
                Ok(val) => val,
                Err(e) => {
                    eprintln!("[daily-news] Storage error checking consumed status for '{}': {e}", c.candidate_id);
                    true
                }
            };
            if !is_consumed && self.reverify_cached_readiness_if_needed(c) {
                updated = true;
            }
        }

        let mut seen_content_hashes = HashSet::new();
        let mut seen_title_hashes = HashSet::new();

        let eligible: Vec<NormalizedCandidate> = all_candidates
            .iter()
            .filter(|c| match self.storage.try_is_consumed(&c.candidate_id) {
                Ok(false) => true,
                Ok(true) => false,
                Err(e) => {
                    eprintln!("[daily-news] Storage error checking consumed status for '{}': {e}", c.candidate_id);
                    false
                }
            })
            .filter(|c| policy.is_candidate_eligible(c, &mut seen_content_hashes, &mut seen_title_hashes))
            .cloned()
            .collect();

        let ranked = self.scorer.rank_candidates(eligible, limit);
        if updated {
            self.storage.save_candidates(&all_candidates)?;
        }
        Ok(ranked)
    }

    pub fn get_candidates(&self, limit: usize) -> Vec<NormalizedCandidate> {
        match self.try_get_candidates(limit) {
            Ok(ranked) => ranked,
            Err(e) => {
                eprintln!("[daily-news] CRITICAL: Persistence error in get_candidates: {e}");
                Vec::new()
            }
        }
    }

    pub fn get_candidate(&self, candidate_id: &str) -> Option<NormalizedCandidate> {
        let mut cand = self.storage.get_candidate(candidate_id)?;
        if self.reverify_cached_readiness_if_needed(&mut cand) {
            if let Err(e) = self.storage.save_candidates(std::slice::from_ref(&cand)) {
                eprintln!("[daily-news] Warning: Failed to persist reverified candidate in get_candidate: {e}");
            }
        }
        Some(cand)
    }

    pub fn read_snapshot_bytes(&self, hash_or_ref: &str) -> Option<Vec<u8>> {
        self.capture
            .as_ref()
            .and_then(|c| c.read_snapshot_bytes(hash_or_ref))
    }

    pub fn consume(&self, ids: &[String]) -> Result<usize, String> {
        self.storage.mark_consumed(ids)
    }

    pub fn get_config(&self) -> ModuleConfig {
        self.storage.load_config()
    }

    pub fn update_config(&self, new_config: ModuleConfig) -> Result<ModuleConfig, String> {
        self.storage.save_config(&new_config)?;
        let new_fetcher = crate::fetcher::build_live_feed_fetcher(&new_config);
        if let Ok(mut g) = self.fetcher.write() {
            *g = Arc::new(new_fetcher);
        }
        Ok(new_config)
    }

    pub fn chat_completion(&self, limit: usize) -> String {
        let candidates = self.get_candidates(limit);
        let mut lines = vec![format!(
            "### Daily News Briefing ({} items):\n",
            candidates.len()
        )];
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

    pub fn read_snapshot_text(&self, hash_or_ref: &str) -> Option<String> {
        self.capture
            .as_ref()
            .and_then(|c| c.read_snapshot_text(hash_or_ref))
    }
}
