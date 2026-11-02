//! Domain models, value objects, and sanitization for Daily News.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawNewsItem {
    pub id: String,
    pub url: String,
    pub title: String,
    pub published_at: String,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub discussion_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedCandidate {
    pub candidate_id: String,
    pub canonical_url: String,
    pub original_url: String,
    pub title: String,
    pub publisher: String,
    pub normalized_publisher: String,
    pub published_at_utc: String,
    pub content_hash: String,
    pub title_hash: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub discussion_url: Option<String>,
    #[serde(default)]
    pub topic_family: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scoring: Option<CandidateScoring>,
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

pub fn canonicalize_url(url: &str) -> String {
    let clean = url.trim();
    let without_fragment = clean.split('#').next().unwrap_or(clean);
    if let Some((base, query)) = without_fragment.split_once('?') {
        let filtered: Vec<&str> = query
            .split('&')
            .filter(|p| {
                let key = p.split('=').next().unwrap_or("").to_lowercase();
                !key.starts_with("utm_")
                    && key != "fbclid"
                    && key != "gclid"
                    && key != "ref"
                    && key != "source"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonicalize_url() {
        assert_eq!(
            canonicalize_url("https://example.com/article/?utm_source=twitter&foo=bar#section"),
            "https://example.com/article?foo=bar"
        );
        assert_eq!(
            canonicalize_url("https://example.com/path/?utm_medium=cpc"),
            "https://example.com/path"
        );
    }

    #[test]
    fn test_compute_sha256() {
        let hash = compute_sha256("hello world");
        assert_eq!(hash.len(), 64);
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_discovery_result_serialization() {
        let result = DiscoveryResult {
            tier: "tier_2_automated".to_string(),
            status: "ELIGIBLE".to_string(),
            count: 2,
            candidates: vec![],
            source_file: None,
            reason: None,
            diagnostics: serde_json::json!({}),
        };
        let json_str = serde_json::to_string(&result).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(val["count"], 2);
        assert_eq!(val["tier"], "tier_2_automated");
        assert_eq!(val["status"], "ELIGIBLE");
    }
}
