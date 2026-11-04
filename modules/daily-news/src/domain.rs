//! Domain models, value objects, and sanitization for Daily News.
//!
//! Pure domain logic matching Clean Architecture rules and strict contracts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use crate::date::*;
pub use crate::research_readiness::*;
pub use crate::scoring::ProductionReadinessScoring;
pub use crate::storage::resolve_state_dir;
pub use crate::url::*;

fn default_newsnow_url() -> String {
    "http://127.0.0.1:4444".to_string()
}
fn default_freshness() -> f64 {
    48.0
}
fn default_max_age_days() -> u32 {
    7
}
fn default_require_latest() -> bool {
    true
}
fn default_sources() -> Vec<String> {
    vec![
        "hackernews".to_string(),
        "reddit".to_string(),
        "github".to_string(),
        "producthunt".to_string(),
    ]
}
fn default_reddit_enabled() -> bool {
    true
}
fn default_reddit_subreddits() -> Vec<String> {
    vec![
        "LocalLLaMA".to_string(),
        "MachineLearning".to_string(),
        "artificial".to_string(),
        "technology".to_string(),
    ]
}
fn default_redis_url() -> String {
    "redis://localhost:6379/0".to_string()
}
fn default_redis_key() -> String {
    "news:incoming".to_string()
}
fn default_redis_type() -> String {
    "stream".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleConfig {
    #[serde(default = "default_newsnow_url")]
    pub newsnow_url: String,
    #[serde(default = "default_freshness")]
    pub freshness_window_hours: f64,
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u32,
    #[serde(default = "default_require_latest")]
    pub require_latest_news: bool,
    #[serde(default = "default_sources")]
    pub default_sources: Vec<String>,
    #[serde(default)]
    pub redis_enabled: bool,
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
    #[serde(default = "default_redis_key")]
    pub redis_key: String,
    #[serde(default = "default_redis_type")]
    pub redis_type: String,
    #[serde(default = "default_reddit_enabled")]
    pub reddit_enabled: bool,
    #[serde(default = "default_reddit_subreddits")]
    pub reddit_subreddits: Vec<String>,
    #[serde(default)]
    pub reddit_client_id: Option<String>,
    #[serde(default)]
    pub reddit_client_secret: Option<String>,
    #[serde(default)]
    pub reddit_base_url: Option<String>,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self {
            newsnow_url: default_newsnow_url(),
            freshness_window_hours: default_freshness(),
            max_age_days: default_max_age_days(),
            require_latest_news: default_require_latest(),
            default_sources: default_sources(),
            redis_enabled: false,
            redis_url: default_redis_url(),
            redis_key: default_redis_key(),
            redis_type: default_redis_type(),
            reddit_enabled: default_reddit_enabled(),
            reddit_subreddits: default_reddit_subreddits(),
            reddit_client_id: None,
            reddit_client_secret: None,
            reddit_base_url: None,
        }
    }
}

impl ModuleConfig {
    pub fn apply_updates_validated(&mut self, body: &serde_json::Value) -> Result<(), String> {
        let obj = body
            .as_object()
            .ok_or_else(|| "Configuration updates payload must be a JSON object".to_string())?;

        if obj.is_empty() {
            return Err("Configuration updates payload cannot be empty".to_string());
        }

        if let Some(val) = obj.get("newsnow_url") {
            let url = val
                .as_str()
                .ok_or_else(|| "'newsnow_url' must be a string".to_string())?;
            if !url.trim().is_empty() && !crate::url::is_safe_url(url, None) {
                return Err(format!("'newsnow_url' is invalid or not an allowed URL: {url}"));
            }
            self.newsnow_url = url.to_string();
        }

        if let Some(val) = obj.get("freshness_window_hours") {
            let h = val
                .as_f64()
                .ok_or_else(|| "'freshness_window_hours' must be a positive number".to_string())?;
            if h <= 0.0 || h > 8760.0 {
                return Err("'freshness_window_hours' must be between 0.1 and 8760.0 hours".to_string());
            }
            self.freshness_window_hours = h;
        }

        if let Some(val) = obj.get("max_age_days") {
            let d = val
                .as_u64()
                .ok_or_else(|| "'max_age_days' must be a positive integer".to_string())?;
            if d == 0 || d > 365 {
                return Err("'max_age_days' must be between 1 and 365 days".to_string());
            }
            self.max_age_days = d as u32;
        }

        if let Some(val) = obj.get("require_latest_news") {
            self.require_latest_news = val
                .as_bool()
                .ok_or_else(|| "'require_latest_news' must be a boolean".to_string())?;
        }

        if let Some(val) = obj.get("redis_enabled") {
            self.redis_enabled = val
                .as_bool()
                .ok_or_else(|| "'redis_enabled' must be a boolean".to_string())?;
        }

        if let Some(val) = obj.get("redis_url") {
            self.redis_url = val
                .as_str()
                .ok_or_else(|| "'redis_url' must be a string".to_string())?
                .to_string();
        }

        if let Some(val) = obj.get("redis_key") {
            self.redis_key = val
                .as_str()
                .ok_or_else(|| "'redis_key' must be a string".to_string())?
                .to_string();
        }

        if let Some(val) = obj.get("redis_type") {
            self.redis_type = val
                .as_str()
                .ok_or_else(|| "'redis_type' must be a string".to_string())?
                .to_string();
        }

        if let Some(val) = obj.get("reddit_enabled") {
            self.reddit_enabled = val
                .as_bool()
                .ok_or_else(|| "'reddit_enabled' must be a boolean".to_string())?;
        }

        if let Some(val) = obj.get("reddit_subreddits") {
            let arr = val
                .as_array()
                .ok_or_else(|| "'reddit_subreddits' must be an array of strings".to_string())?;
            let mut subs = Vec::new();
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or_else(|| "'reddit_subreddits' elements must be strings".to_string())?;
                if !s.trim().is_empty() {
                    subs.push(s.trim().to_string());
                }
            }
            self.reddit_subreddits = subs;
        }

        if let Some(val) = obj.get("reddit_client_id") {
            self.reddit_client_id = val.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }

        if let Some(val) = obj.get("reddit_client_secret") {
            self.reddit_client_secret = val.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }

        if let Some(val) = obj.get("reddit_base_url") {
            self.reddit_base_url = val.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }

        Ok(())
    }

    pub fn apply_updates(&mut self, body: &serde_json::Value) {
        let _ = self.apply_updates_validated(body);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringAxes {
    pub freshness: f64,
    pub publisher_authority: f64,
    pub fact_density: f64,
    pub visual_potential: f64,
    pub audience_interest: f64,
    pub feasibility: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateScoring {
    pub total_score: f64,
    pub axes: ScoringAxes,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoryboardEvidence {
    #[serde(default)]
    pub hook: Option<String>,
    #[serde(default)]
    pub viewer_question: Option<String>,
    #[serde(default)]
    pub symptom: Option<String>,
    #[serde(default)]
    pub root_cause: Option<String>,
    #[serde(default)]
    pub remedy: Option<String>,
    #[serde(default)]
    pub evidence_excerpt: Option<String>,
    #[serde(default)]
    pub source_snapshot: Option<String>,
    #[serde(default)]
    pub snapshot_sha256: Option<String>,
}

impl StoryboardEvidence {
    pub fn from_json_map(obj: &serde_json::Map<String, serde_json::Value>) -> Self {
        let get_str = |k: &str| obj.get(k).and_then(|v| v.as_str()).map(str::to_string);
        Self {
            hook: get_str("hook"),
            viewer_question: get_str("viewer_question"),
            symptom: get_str("symptom"),
            root_cause: get_str("root_cause"),
            remedy: get_str("remedy"),
            evidence_excerpt: get_str("evidence_excerpt"),
            source_snapshot: get_str("source_snapshot"),
            snapshot_sha256: get_str("snapshot_sha256"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawProvenance {
    pub source_id: String,
    pub source_feed: String,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for RawProvenance {
    fn default() -> Self {
        Self {
            source_id: String::new(),
            source_feed: "daily-news".to_string(),
            extra: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNewsItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub published_at: String,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub discussion_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub is_evergreen: bool,
    #[serde(default)]
    pub content_kind: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(flatten)]
    pub evidence: StoryboardEvidence,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub content_origin: ContentOrigin,
    #[serde(default)]
    pub published_at_kind: PublishedAtKind,
    #[serde(flatten)]
    pub research: ResearchEvidenceFields,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResearchEvidenceFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub article_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discussion: Option<DiscussionPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_source: Option<PrimarySourceEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<ClaimEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visual_evidence: Vec<VisualEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_readiness: Option<ResearchReadiness>,
}

impl std::ops::Deref for RawNewsItem {
    type Target = ResearchEvidenceFields;
    fn deref(&self) -> &Self::Target {
        &self.research
    }
}

impl std::ops::DerefMut for RawNewsItem {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.research
    }
}

impl RawNewsItem {
    pub fn new(
        id: impl Into<String>,
        url: impl Into<String>,
        title: impl Into<String>,
        published_at: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            title: title.into(),
            published_at: published_at.into(),
            publisher: None,
            author: None,
            description: None,
            body: None,
            discussion_url: None,
            tags: Vec::new(),
            is_evergreen: false,
            content_kind: None,
            updated_at: None,
            evidence: StoryboardEvidence::default(),
            extra: HashMap::new(),
            content_origin: ContentOrigin::Unknown,
            published_at_kind: PublishedAtKind::Unknown,
            research: ResearchEvidenceFields::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NormalizedCandidate {
    pub candidate_id: String,
    pub canonical_url: String,
    pub original_url: String,
    pub url: String,
    pub source_url: String,
    pub source_type: String,
    pub title: String,
    pub publisher: String,
    pub normalized_publisher: String,
    pub published_at_utc: String,
    pub published_at: String,
    pub content_hash: String,
    pub title_hash: String,
    pub raw_provenance: RawProvenance,
    pub adapter_version: String,
    pub topic_family: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub is_evergreen: bool,
    #[serde(default = "default_content_kind")]
    pub content_kind: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub discussion_url: Option<String>,
    #[serde(flatten)]
    pub evidence: StoryboardEvidence,
    #[serde(default)]
    pub independent_fact_check: Option<serde_json::Value>,
    #[serde(default)]
    pub scoring: Option<CandidateScoring>,
    #[serde(default)]
    pub content_origin: ContentOrigin,
    #[serde(default = "default_published_at_kind")]
    pub published_at_kind: String,
    #[serde(flatten)]
    pub research: ResearchEvidenceFields,
}

impl std::ops::Deref for NormalizedCandidate {
    type Target = ResearchEvidenceFields;
    fn deref(&self) -> &Self::Target {
        &self.research
    }
}

impl std::ops::DerefMut for NormalizedCandidate {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.research
    }
}

fn default_published_at_kind() -> String {
    "unknown".to_string()
}

fn default_language() -> String {
    "en".to_string()
}
fn default_content_kind() -> String {
    "news".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryResult {
    pub tier: String,
    pub status: String,
    pub count: usize,
    pub candidates: Vec<NormalizedCandidate>,
    #[serde(default)]
    pub source_file: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub diagnostics: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscussionComment {
    pub id: String,
    pub author: Option<String>,
    pub body: String,
    pub score: i64,
    pub created_utc: u64,
    pub created_at: String,
    pub permalink: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<DiscussionComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscussionThread {
    pub id: String,
    pub subreddit: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub permalink: String,
    pub url: Option<String>,
    pub selftext: Option<String>,
    pub score: i64,
    pub num_comments: i64,
    pub comments: Vec<DiscussionComment>,
}
