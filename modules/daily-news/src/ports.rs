//! Outbound Port interfaces for Daily News.
//!
//! Follows Clean Architecture Dependency Inversion Principle (DIP):
//! Core application services depend on these traits, while external
//! HTTP fetchers and storage adapters implement them.

use crate::domain::{DiscussionThread, ModuleConfig, NormalizedCandidate, RawNewsItem};

pub trait DiscussionFetcherPort: Send + Sync {
    /// Inspect a discussion thread (Reddit or Hacker News) by URL or item ID.
    fn fetch_thread(
        &self,
        url_or_id: &str,
        limit: Option<usize>,
    ) -> Result<DiscussionThread, String>;
}

/// In-memory no-op discussion fetcher used for testing and decoupled service construction.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopDiscussionFetcher;

impl DiscussionFetcherPort for NoopDiscussionFetcher {
    fn fetch_thread(
        &self,
        _url_or_id: &str,
        _limit: Option<usize>,
    ) -> Result<DiscussionThread, String> {
        Err("No discussion fetcher configured".to_string())
    }
}

pub trait FeedFetcherPort: Send + Sync {
    /// Ingest raw news items from external feeds (Hacker News, NewsNow, etc.).
    fn fetch_raw_items(&self, limit: usize) -> Result<Vec<RawNewsItem>, String>;

    /// Ingest items from specified sources (e.g. hackernews, redis, newsnow).
    fn fetch_sources(&self, _sources: &[String], limit: usize) -> Result<Vec<RawNewsItem>, String> {
        self.fetch_raw_items(limit)
    }
}

pub trait StoragePort: Send + Sync {
    /// Check if a candidate ID has already been consumed/processed.
    fn is_consumed(&self, candidate_id: &str) -> bool;

    /// Mark a batch of candidate IDs as consumed.
    fn mark_consumed(&self, candidate_ids: &[String]) -> Result<usize, String>;

    /// Count how many items have been marked consumed.
    fn consumed_count(&self) -> usize;

    /// Load the active module configuration.
    fn load_config(&self) -> ModuleConfig;

    /// Save updated module configuration.
    fn save_config(&self, config: &ModuleConfig) -> Result<(), String>;

    /// Cache normalized candidates for quick access.
    fn cache_candidates(&self, candidates: &[NormalizedCandidate]);

    /// Retrieve cached candidates.
    fn get_cached_candidates(&self) -> Vec<NormalizedCandidate>;
}

pub trait SourceCapturePort: Send + Sync {
    /// Capture an external primary source URL into an immutable, hashed snapshot.
    fn capture_source(&self, url: &str) -> Result<crate::domain::PrimarySourceEvidence, String>;
}
