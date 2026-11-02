//! Application service orchestrating news ingestion, ranking, and tiered discovery use cases.
//!
//! Follows Clean Architecture Application Layer principles:
//! Ingests real-time live news feeds, enforces cross-source deduplication,
//! applies 6-axis ranking, and manages persistent consumption tracking.

use crate::date::{parse_boundary_datetime, parse_iso_to_epoch};
use crate::domain::{
    canonicalize_url, classify_topic_family, compute_content_hash, compute_title_hash,
    is_safe_url, normalize_publisher, DiscoveryResult, ModuleConfig, NormalizedCandidate,
    RawNewsItem, RawProvenance,
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

    pub fn normalize_item(&self, raw: &RawNewsItem) -> Result<NormalizedCandidate, String> {
        if raw.id.trim().is_empty() {
            return Err("Raw item missing required 'id'".to_string());
        }
        if raw.url.trim().is_empty() {
            return Err("Raw item missing required 'url'".to_string());
        }
        if raw.title.trim().is_empty() {
            return Err("Raw item missing required 'title'".to_string());
        }
        if raw.published_at.trim().is_empty() {
            return Err("Raw item missing required 'published_at'".to_string());
        }
        if !is_safe_url(&raw.url, None) {
            return Err(format!("URL violates security boundary: {}", raw.url));
        }

        let canonical = canonicalize_url(&raw.url);
        let norm_pub = normalize_publisher(raw.publisher.as_deref());
        let disc_url = raw.discussion_url.clone().or_else(|| {
            if raw.url.contains("news.ycombinator.com") {
                Some(raw.url.clone())
            } else {
                None
            }
        });

        let body_str = raw.body.as_deref().or(raw.description.as_deref());
        let chash = compute_content_hash(&raw.title, body_str, &canonical);
        let thash = compute_title_hash(&raw.title);
        let topic = classify_topic_family(&raw.title, body_str.unwrap_or(""));

        Ok(NormalizedCandidate {
            candidate_id: format!("cand_{}", &chash[..16]),
            canonical_url: canonical.clone(),
            original_url: raw.url.clone(),
            url: canonical,
            title: raw.title.clone(),
            publisher: raw.publisher.clone().unwrap_or_else(|| norm_pub.clone()),
            normalized_publisher: norm_pub,
            published_at_utc: raw.published_at.clone(),
            published_at: raw.published_at.clone(),
            content_hash: chash,
            title_hash: thash,
            raw_provenance: RawProvenance {
                source_id: raw.id.clone(),
                source_feed: "daily-news".to_string(),
                extra: raw.extra.clone(),
            },
            adapter_version: "1.3.0".to_string(),
            topic_family: topic,
            author: raw.author.clone(),
            description: raw.description.clone(),
            body: raw.body.clone(),
            summary: raw.description.clone().or_else(|| raw.body.clone()),
            tags: raw.tags.clone(),
            language: "en".to_string(),
            image_url: None,
            is_evergreen: raw.is_evergreen,
            content_kind: raw.content_kind.clone().unwrap_or_else(|| "news".to_string()),
            updated_at: raw.updated_at.clone(),
            discussion_url: disc_url,
            independent_fact_check: None,
            scoring: None,
        })
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

            let effective_ts = cand.updated_at.as_deref().unwrap_or(&cand.published_at_utc);
            let effective_epoch = parse_iso_to_epoch(effective_ts).unwrap_or(0);

            if effective_epoch > upper_bound {
                continue;
            }

            let enforce_latest = cfg.require_latest_news || from_epoch.is_some();
            if effective_epoch < cutoff && (enforce_latest || !cand.is_evergreen) {
                continue;
            }

            if !seen_content_hashes.insert(cand.content_hash.clone()) {
                continue;
            }
            if !seen_title_hashes.insert(cand.title_hash.clone()) {
                continue;
            }

            if let Some(topic_filter) = topics {
                let targets: Vec<String> = topic_filter
                    .iter()
                    .map(|t| t.trim().to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect();
                if !targets.is_empty() {
                    let fam = cand.topic_family.to_lowercase();
                    let title = cand.title.to_lowercase();
                    let matched = targets.iter().any(|t| {
                        fam == *t
                            || fam.contains(t)
                            || title.contains(t)
                            || cand.tags.iter().any(|tag| tag.to_lowercase() == *t)
                    });
                    if !matched {
                        continue;
                    }
                }
            }

            unconsumed.push(cand);
        }

        self.scorer.rank_candidates(unconsumed, limit)
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
        let from_epoch = if let Some(fd) = from_date_str {
            match parse_boundary_datetime(fd, false) {
                Ok(e) => Some(e),
                Err(err) => return Err(("INVALID_DATE_FORMAT".to_string(), format!("from_date error: {err}"))),
            }
        } else {
            None
        };

        let to_epoch = if let Some(td) = to_date_str {
            match parse_boundary_datetime(td, true) {
                Ok(e) => Some(e),
                Err(err) => return Err(("INVALID_DATE_FORMAT".to_string(), format!("to_date error: {err}"))),
            }
        } else {
            None
        };

        if let (Some(f), Some(t)) = (from_epoch, to_epoch) {
            if f > t {
                return Err((
                    "INVALID_DATE_RANGE".to_string(),
                    format!("from_date ({from_date_str:?}) must not be later than to_date ({to_date_str:?})"),
                ));
            }
        }

        // --- Tier 1: Curated candidates ---
        if let Some(curated_feed) = curated {
            if !curated_feed.is_empty() {
                let ranked = self.filter_and_score(&curated_feed, from_epoch, to_epoch, topics, count);
                if !ranked.is_empty() {
                    let c_len = ranked.len();
                    return Ok(DiscoveryResult {
                        tier: "tier_1_curated".to_string(),
                        status: "ELIGIBLE".to_string(),
                        count: c_len,
                        candidates: ranked,
                        source_file: Some("curated_feed".to_string()),
                        reason: None,
                        diagnostics: json!({ "tier": "tier_1_curated" }),
                    });
                }
            }
        }

        // --- Tier 2: Automated live feeds ---
        let raw = if let Some(ref s) = sources {
            self.fetcher.fetch_sources(s, count.max(30)).unwrap_or_default()
        } else {
            self.fetcher.fetch_raw_items(count.max(30)).unwrap_or_default()
        };

        let ranked = self.filter_and_score(&raw, from_epoch, to_epoch, topics, count);
        if !ranked.is_empty() {
            self.storage.cache_candidates(&ranked);
            let c_len = ranked.len();
            return Ok(DiscoveryResult {
                tier: "tier_2_automated".to_string(),
                status: "ELIGIBLE".to_string(),
                count: c_len,
                candidates: ranked,
                source_file: Some("live_feeds".to_string()),
                reason: None,
                diagnostics: json!({ "tier": "tier_2_automated" }),
            });
        }

        // --- Tier 2 Fallback: Cached candidates ---
        let cached = self.storage.get_cached_candidates();
        let mut unconsumed_cached: Vec<NormalizedCandidate> = cached
            .into_iter()
            .filter(|c| !self.storage.is_consumed(&c.candidate_id))
            .collect();

        if let Some(topic_filter) = topics {
            let targets: Vec<String> = topic_filter
                .iter()
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect();
            if !targets.is_empty() {
                unconsumed_cached.retain(|cand| {
                    let fam = cand.topic_family.to_lowercase();
                    let title = cand.title.to_lowercase();
                    targets.iter().any(|t| {
                        fam == *t
                            || fam.contains(t)
                            || title.contains(t)
                            || cand.tags.iter().any(|tag| tag.to_lowercase() == *t)
                    })
                });
            }
        }

        if let Some(from_e) = from_epoch {
            unconsumed_cached.retain(|c| {
                let ts = c.updated_at.as_deref().unwrap_or(&c.published_at_utc);
                parse_iso_to_epoch(ts).map(|ep| ep >= from_e).unwrap_or(false)
            });
        }
        if let Some(to_e) = to_epoch {
            unconsumed_cached.retain(|c| {
                let ts = c.updated_at.as_deref().unwrap_or(&c.published_at_utc);
                parse_iso_to_epoch(ts).map(|ep| ep <= to_e).unwrap_or(false)
            });
        }

        if !unconsumed_cached.is_empty() {
            let c_len = unconsumed_cached.len().min(count);
            return Ok(DiscoveryResult {
                tier: "tier_2_cache".to_string(),
                status: "ELIGIBLE".to_string(),
                count: c_len,
                candidates: unconsumed_cached.into_iter().take(count).collect(),
                source_file: Some("cached_candidates".to_string()),
                reason: None,
                diagnostics: json!({ "tier": "tier_2_cache" }),
            });
        }

        // --- Tier 3: Fail-Closed ---
        let mut fail_reasons = Vec::new();
        if let Some(t) = topics {
            fail_reasons.push(format!("topic filter(s): {:?}", t));
        }
        if from_date_str.is_some() || to_date_str.is_some() {
            fail_reasons.push(format!("date range: [{:?}, {:?}]", from_date_str, to_date_str));
        }

        let reason = if fail_reasons.is_empty() {
            "All candidates were consumed, stale, or dropped by safety/domain policies".to_string()
        } else {
            format!("No eligible candidates matched the requested {}", fail_reasons.join(", "))
        };

        Ok(DiscoveryResult {
            tier: "none".to_string(),
            status: "NO_ELIGIBLE_TOPIC".to_string(),
            count: 0,
            candidates: vec![],
            source_file: None,
            reason: Some(reason),
            diagnostics: json!({ "tier_3_fail_closed": true }),
        })
    }

    pub fn get_feed(&self, limit: usize) -> Vec<RawNewsItem> {
        self.fetcher.fetch_raw_items(limit).unwrap_or_default()
    }

    pub fn get_feed_sources(&self, sources: &[String], limit: usize) -> Vec<RawNewsItem> {
        self.fetcher.fetch_sources(sources, limit).unwrap_or_default()
    }

    pub fn get_candidates(&self, limit: usize) -> Vec<NormalizedCandidate> {
        let raw = self.get_feed(limit.max(25));
        let candidates: Vec<NormalizedCandidate> = raw.iter().filter_map(|r| self.normalize_item(r).ok()).collect();
        let ranked = self.scorer.rank_candidates(candidates, limit);
        self.storage.cache_candidates(&ranked);
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
