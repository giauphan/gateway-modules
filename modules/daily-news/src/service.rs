//! Application service orchestrating news ingestion, ranking, and tiered discovery use cases.
//!
//! Follows Clean Architecture Application Layer principles:
//! Ingests real-time live news feeds, enforces cross-source deduplication,
//! applies 6-axis ranking, and manages persistent consumption tracking.

use crate::date::parse_iso_to_epoch;
use crate::discovery::{
    build_fail_closed_result, is_candidate_eligible, matches_topics, normalize_raw_item,
    validate_date_bounds,
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

pub struct DailyNewsService {
    fetcher: Arc<dyn FeedFetcherPort>,
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
            fetcher,
            storage,
            scorer,
            discussion: Arc::new(NoopDiscussionFetcher),
            capture: None,
        }
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

        normalize_raw_item(
            raw,
            snapshot_text.as_deref(),
            Some(&secondary_reader),
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
        let ref_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1793618000);

        let cutoff = from_epoch.unwrap_or_else(|| {
            let window_secs = (cfg.freshness_window_hours * 3600.0) as u64;
            ref_epoch.saturating_sub(window_secs)
        });
        let upper_bound = to_epoch.unwrap_or(ref_epoch + 300);

        let mut seen_content_hashes = HashSet::new();
        let mut seen_title_hashes = HashSet::new();
        let mut unconsumed = Vec::new();

        for r in raw_items {
            let cand = match self.normalize_item(r) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if self.storage.is_consumed(&cand.candidate_id) {
                continue;
            }

            if !is_candidate_eligible(
                &cand,
                &cfg,
                from_epoch,
                cutoff,
                upper_bound,
                &mut seen_content_hashes,
                &mut seen_title_hashes,
                topics,
            ) {
                continue;
            }

            unconsumed.push(cand);
        }

        self.scorer.rank_candidates(unconsumed, limit)
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
        if ranked.is_empty() {
            return Ok(None);
        }
        self.storage
            .save_candidates(&ranked)
            .map_err(|e| ("PERSISTENCE_ERROR".to_string(), format!("Failed to persist candidates: {e}")))?;
        let c_len = ranked.len();
        Ok(Some(DiscoveryResult {
            tier: "tier_1_curated".to_string(),
            status: "ELIGIBLE".to_string(),
            count: c_len,
            candidates: ranked,
            source_file: Some("curated_feed".to_string()),
            reason: None,
            diagnostics: json!({ "tier": "tier_1_curated" }),
        }))
    }

    fn try_live_tier(
        &self,
        sources: Option<&[String]>,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        topics: Option<&[String]>,
        count: usize,
    ) -> Result<Option<DiscoveryResult>, (String, String)> {
        let raw = match sources {
            Some(s) => self
                .fetcher
                .fetch_sources(s, count.max(30))
                .unwrap_or_default(),
            None => self
                .fetcher
                .fetch_raw_items(count.max(30))
                .unwrap_or_default(),
        };
        let ranked = self.filter_and_score(&raw, from_epoch, to_epoch, topics, count);
        if ranked.is_empty() {
            return Ok(None);
        }
        self.storage
            .save_candidates(&ranked)
            .map_err(|e| ("PERSISTENCE_ERROR".to_string(), format!("Failed to persist candidates: {e}")))?;
        let c_len = ranked.len();
        Ok(Some(DiscoveryResult {
            tier: "tier_2_automated".to_string(),
            status: "ELIGIBLE".to_string(),
            count: c_len,
            candidates: ranked,
            source_file: Some("live_feeds".to_string()),
            reason: None,
            diagnostics: json!({ "tier": "tier_2_automated" }),
        }))
    }

    fn try_cache_tier(
        &self,
        from_epoch: Option<u64>,
        to_epoch: Option<u64>,
        topics: Option<&[String]>,
        count: usize,
    ) -> Option<DiscoveryResult> {
        let cached = self.storage.get_cached_candidates();
        let mut unconsumed: Vec<NormalizedCandidate> = cached
            .into_iter()
            .filter(|c| !self.storage.is_consumed(&c.candidate_id))
            .collect();

        if let Some(topic_filter) = topics {
            unconsumed.retain(|cand| matches_topics(cand, topic_filter));
        }
        if let Some(from_e) = from_epoch {
            unconsumed.retain(|c| {
                let ts = c.updated_at.as_deref().unwrap_or(&c.published_at_utc);
                parse_iso_to_epoch(ts).is_ok_and(|ep| ep >= from_e)
            });
        }
        if let Some(to_e) = to_epoch {
            unconsumed.retain(|c| {
                let ts = c.updated_at.as_deref().unwrap_or(&c.published_at_utc);
                parse_iso_to_epoch(ts).is_ok_and(|ep| ep <= to_e)
            });
        }

        if unconsumed.is_empty() {
            return None;
        }
        let c_len = unconsumed.len().min(count);
        Some(DiscoveryResult {
            tier: "tier_2_cache".to_string(),
            status: "ELIGIBLE".to_string(),
            count: c_len,
            candidates: unconsumed.into_iter().take(count).collect(),
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
        let mut items = self.fetcher.fetch_raw_items(limit).unwrap_or_default();
        items.truncate(limit);
        items
    }

    pub fn get_feed_sources(&self, sources: &[String], limit: usize) -> Vec<RawNewsItem> {
        let mut items = self
            .fetcher
            .fetch_sources(sources, limit)
            .unwrap_or_default();
        items.truncate(limit);
        items
    }

    pub fn get_candidates(&self, limit: usize) -> Vec<NormalizedCandidate> {
        let raw = self.get_feed(limit.max(25));
        let candidates: Vec<NormalizedCandidate> = raw
            .iter()
            .filter_map(|r| self.normalize_item(r).ok())
            .collect();
        let ranked = self.scorer.rank_candidates(candidates, limit);
        // Propagate persistence error as eprintln; fall back to cached candidates on failure
        // so callers (chat_completion, MCP dispatch) always get a usable list.
        if let Err(e) = self.storage.save_candidates(&ranked) {
            eprintln!(
                "[daily-news] WARNING: get_candidates persistence failed, \
                 serving stale cache. Error: {e}"
            );
            return self.storage.get_cached_candidates();
        }
        ranked
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

    pub fn get_candidate(&self, candidate_id: &str) -> Option<NormalizedCandidate> {
        self.storage.get_candidate(candidate_id)
    }

    pub fn read_snapshot_text(&self, hash_or_ref: &str) -> Option<String> {
        self.capture
            .as_ref()
            .and_then(|c| c.read_snapshot_text(hash_or_ref))
    }
}
