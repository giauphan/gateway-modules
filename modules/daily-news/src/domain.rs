//! Domain models, value objects, and sanitization for Daily News.
//!
//! Pure domain logic matching Clean Architecture rules and strict contracts.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;

pub use crate::date::*;

pub const DEFAULT_ALLOWED_DOMAINS: &[&str] = &[
    "docs.anthropic.com",
    "docs.github.com",
    "modelcontextprotocol.io",
    "microsoft.github.io",
    "openai.com",
    "arxiv.org",
    "github.com",
    "git-scm.com",
    "news.ycombinator.com",
    "producthunt.com",
    "techcrunch.com",
    "theverge.com",
    "wired.com",
    "arstechnica.com",
    "venturebeat.com",
];

pub const TRACKING_QUERY_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "msclkid",
    "ref",
    "source",
    "rss",
];

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
        "github".to_string(),
        "producthunt".to_string(),
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
        }
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub fn extract_domain(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = if let Some(stripped) = trimmed.strip_prefix("https://") {
        stripped
    } else if let Some(stripped) = trimmed.strip_prefix("http://") {
        stripped
    } else {
        trimmed
    };
    let host_part = without_scheme.split('/').next().unwrap_or(without_scheme);
    let domain = host_part.split(':').next().unwrap_or(host_part);
    domain.trim_start_matches("www.").to_lowercase()
}

pub fn normalize_publisher(publisher: Option<&str>) -> String {
    match publisher {
        None => "Unknown".to_string(),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return "Unknown".to_string();
            }
            let mut cleaned = trimmed.to_string();
            for suffix in &[".com", ".org", ".net", ".co", ".io", ".gov", ".news", ".sh"] {
                if cleaned.to_lowercase().ends_with(suffix) {
                    cleaned.truncate(cleaned.len() - suffix.len());
                    break;
                }
            }
            let out = cleaned.trim();
            if out.is_empty() {
                "Unknown".to_string()
            } else {
                out.to_string()
            }
        }
    }
}

pub fn canonicalize_url(url: &str) -> String {
    let clean = url.trim();
    if clean.is_empty() {
        return String::new();
    }
    let without_fragment = clean.split('#').next().unwrap_or(clean);
    if let Some((base, query)) = without_fragment.split_once('?') {
        let filtered: Vec<&str> = query
            .split('&')
            .filter(|p| {
                let key = p.split('=').next().unwrap_or("").to_lowercase();
                !TRACKING_QUERY_PARAMS
                    .iter()
                    .any(|&tp| key == tp || key.starts_with("utm_"))
            })
            .collect();
        if filtered.is_empty() {
            base.trim_end_matches('/').to_string()
        } else {
            format!("{}?{}", base.trim_end_matches('/'), filtered.join("&"))
        }
    } else {
        without_fragment.trim_end_matches('/').to_string()
    }
}

pub fn compute_sha256(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in result {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub fn compute_title_hash(title: &str) -> String {
    let norm: String = title
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();
    compute_sha256(&norm)
}

pub fn compute_content_hash(title: &str, body: Option<&str>, canonical_url: &str) -> String {
    let clean_title: String = title
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();
    let clean_url = canonical_url.trim().to_lowercase();
    let body_str = body.unwrap_or("").trim();
    let limit = body_str.chars().take(500).collect::<String>();
    let clean_body: String = limit
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();
    let payload = format!("{clean_title}|{clean_url}|{clean_body}");
    compute_sha256(&payload)
}

pub fn classify_topic_family(title: &str, text: &str) -> String {
    let combined = format!("{title} {text}").to_lowercase();
    if [
        "prompt caching",
        "context window",
        "kv cache",
        "memory",
        "state loss",
        "token limit",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "context_memory".to_string();
    }
    if [
        "tool call",
        "mcp",
        "schema",
        "function call",
        "tools",
        "payload parsing",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "agent_tool_use".to_string();
    }
    if [
        "debug",
        "retry",
        "flaky",
        "reliability",
        "ci",
        "loop",
        "crash",
        "failure",
        "trace",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "debugging_reliability".to_string();
    }
    if [
        "benchmark",
        "eval",
        "evaluation",
        "score",
        "metrics",
        "test suite",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "agent_evaluation".to_string();
    }
    "agent_architecture".to_string()
}

pub fn is_safe_url(url: &str, allowed_domains: Option<&HashSet<String>>) -> bool {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return false;
    }
    let host = extract_domain(trimmed);
    if host.is_empty() {
        return false;
    }

    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let octets = ip.octets();
        if octets[0] == 127 {
            return true;
        }
        if octets[0] == 10
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            || (octets[0] == 192 && octets[1] == 168)
            || (octets[0] == 169 && octets[1] == 254)
            || octets[0] == 0
        {
            return false;
        }
    }

    if let Some(allowed) = allowed_domains {
        return allowed
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")));
    }
    true
}
