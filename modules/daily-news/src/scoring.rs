//! Deterministic 6-axis candidate scoring and ranking.

use crate::date::parse_iso_to_epoch;
use crate::domain::{CandidateScoring, NormalizedCandidate, ScoringAxes};

pub const TIER_1_PUBLISHERS: &[&str] = &[
    "reuters",
    "ap",
    "bloomberg",
    "wsj",
    "ft",
    "nytimes",
    "washingtonpost",
    "anthropic",
    "openai",
];

pub const TIER_2_PUBLISHERS: &[&str] = &[
    "theverge",
    "techcrunch",
    "wired",
    "arstechnica",
    "bbc",
    "npr",
    "cnbc",
    "guardian",
    "github",
    "hackernews",
    "ycombinator",
    "producthunt",
];

pub const VISUAL_KEYWORDS: &[&str] = &[
    "chart",
    "graph",
    "photo",
    "video",
    "announced",
    "design",
    "prototype",
    "revealed",
    "leaked",
    "interface",
    "hardware",
    "demo",
    "release",
];

pub const INTEREST_KEYWORDS: &[&str] = &[
    "ai",
    "artificial intelligence",
    "tech",
    "technology",
    "software",
    "hardware",
    "robot",
    "robotics",
    "machine learning",
    "deep learning",
    "llm",
    "model",
    "gpt",
    "claude",
    "gemini",
    "openai",
    "google",
    "microsoft",
    "apple",
    "nvidia",
    "chip",
    "semiconductor",
    "cyber",
    "security",
    "code",
    "coding",
    "developer",
    "app",
    "application",
    "data",
    "computing",
    "algorithm",
    "startup",
    "web",
    "digital",
    "cloud",
    "quantum",
    "browser",
    "linux",
    "open-source",
    "git",
    "api",
    "rust",
    "python",
    "agent",
    "agents",
];

#[derive(Default)]
pub struct CandidateScorer {
    pub reference_time_epoch: Option<u64>,
}

impl CandidateScorer {
    pub fn new(reference_epoch: Option<u64>) -> Self {
        Self {
            reference_time_epoch: reference_epoch,
        }
    }

    fn current_epoch(&self) -> u64 {
        self.reference_time_epoch.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(1793618000)
        })
    }

    pub fn score_freshness(&self, pub_epoch: u64) -> f64 {
        let now = self.current_epoch();
        let age_hours = if now >= pub_epoch {
            (now - pub_epoch) as f64 / 3600.0
        } else {
            0.0
        };

        if age_hours <= 2.0 {
            1.0
        } else if age_hours <= 12.0 {
            1.0 - 0.5 * ((age_hours - 2.0) / 10.0)
        } else if age_hours <= 24.0 {
            0.5 - 0.3 * ((age_hours - 12.0) / 12.0)
        } else {
            0.1
        }
    }

    pub fn score_freshness_with_kind(&self, pub_epoch: u64, kind: Option<&str>) -> f64 {
        if let Some(k) = kind {
            let k_lower = k.to_lowercase();
            if k_lower == "feed_observed" || k_lower == "unknown" {
                return 0.40;
            }
        }
        self.score_freshness(pub_epoch)
    }

    pub fn score_publisher(&self, norm_pub: &str) -> f64 {
        let p = norm_pub.to_lowercase();
        let p_trimmed = p.trim();
        let p_compact = p_trimmed.replace(' ', "");
        if TIER_1_PUBLISHERS
            .iter()
            .any(|&t| p_trimmed == t || p_compact == t)
        {
            1.0
        } else if TIER_2_PUBLISHERS
            .iter()
            .any(|&t| p_trimmed == t || p_compact == t)
        {
            0.8
        } else if !p_trimmed.is_empty() && p_trimmed != "unknown" {
            0.5
        } else {
            0.2
        }
    }

    pub fn score_fact_density(&self, title: &str, body: &str) -> f64 {
        let text = format!("{title} {body}");
        let word_count = text.split_whitespace().count();
        let base: f64 = if word_count > 400 {
            1.0
        } else if word_count > 200 {
            0.8
        } else if word_count > 50 {
            0.5
        } else {
            0.3
        };

        let has_quotes = text.contains('"') || text.contains('\'');
        let has_numbers = text.chars().any(|c| c.is_ascii_digit());
        let bonus: f64 =
            (if has_quotes { 0.1 } else { 0.0 }) + (if has_numbers { 0.1 } else { 0.0 });
        (base + bonus).min(1.0)
    }

    pub fn score_visualizability(&self, title: &str, body: &str, has_image: bool) -> f64 {
        let text = format!("{title} {body}").to_lowercase();
        let mut score: f64 = 0.3 + if has_image { 0.3 } else { 0.0 };
        let matches = VISUAL_KEYWORDS
            .iter()
            .filter(|&&kw| text.contains(kw))
            .count();
        if matches >= 2 {
            score += 0.4;
        } else if matches == 1 {
            score += 0.2;
        }
        score.min(1.0)
    }

    pub fn score_audience_interest(&self, title: &str, body: &str) -> f64 {
        let text = format!("{title} {body}").to_lowercase();
        let matches = INTEREST_KEYWORDS
            .iter()
            .filter(|&&kw| text.contains(kw))
            .count();
        if matches == 0 {
            0.1
        } else if matches >= 2 {
            1.0
        } else {
            0.75
        }
    }

    pub fn score_candidate(&self, candidate: &NormalizedCandidate) -> CandidateScoring {
        let pub_epoch = parse_iso_to_epoch(&candidate.published_at_utc).unwrap_or(0);
        let freshness = self.score_freshness_with_kind(pub_epoch, Some(&candidate.published_at_kind));
        let publisher = self.score_publisher(&candidate.normalized_publisher);

        let body_str = candidate
            .body
            .as_deref()
            .or(candidate.description.as_deref())
            .unwrap_or("");
        let fact_density = if candidate.content_origin == crate::domain::ContentOrigin::HeadlineOnly
            || candidate.content_origin == crate::domain::ContentOrigin::Discussion
            || candidate.content_origin == crate::domain::ContentOrigin::Unknown
        {
            0.2
        } else {
            self.score_fact_density(&candidate.title, body_str)
        };
        let has_genuine_visual =
            candidate.image_url.is_some() || !candidate.visual_evidence.is_empty();
        let visual = self.score_visualizability(&candidate.title, body_str, has_genuine_visual);

        let interest = {
            let kw_score = self.score_audience_interest(&candidate.title, body_str);
            let upvotes = candidate
                .raw_provenance
                .extra
                .get("score")
                .and_then(|v| v.as_f64());
            let comments = candidate
                .raw_provenance
                .extra
                .get("comments")
                .and_then(|v| v.as_f64());
            if let (Some(u), Some(c)) = (upvotes, comments) {
                let engagement = (u + c * 2.0) / 100.0;
                ((kw_score * 0.7) + (engagement.clamp(0.2, 1.0) * 0.3)).min(1.0)
            } else {
                kw_score
            }
        };

        let feasibility = if let Some(ref readiness) = candidate.research_readiness {
            match readiness.status {
                crate::domain::ReadinessStatus::VideoReady => 1.0,
                crate::domain::ReadinessStatus::NeedsVerification => 0.8,
                crate::domain::ReadinessStatus::NeedsResearch => 0.6,
                crate::domain::ReadinessStatus::NeedsSource => 0.3,
                crate::domain::ReadinessStatus::NeedsVisualEvidence => 0.5,
                crate::domain::ReadinessStatus::Discovered => 0.4,
                crate::domain::ReadinessStatus::Rejected => 0.0,
            }
        } else if candidate.primary_source.is_some() {
            0.8
        } else if candidate.content_origin == crate::domain::ContentOrigin::HeadlineOnly
            || candidate.content_origin == crate::domain::ContentOrigin::Unknown
        {
            0.4
        } else {
            0.6
        };
        let total = (freshness * 0.30)
            + (publisher * 0.20)
            + (fact_density * 0.20)
            + (visual * 0.15)
            + (interest * 0.10)
            + (feasibility * 0.05);

        CandidateScoring {
            total_score: (total * 10000.0).round() / 10000.0,
            axes: ScoringAxes {
                freshness: (freshness * 10000.0).round() / 10000.0,
                publisher_authority: (publisher * 10000.0).round() / 10000.0,
                fact_density: (fact_density * 10000.0).round() / 10000.0,
                visual_potential: (visual * 10000.0).round() / 10000.0,
                audience_interest: (interest * 10000.0).round() / 10000.0,
                feasibility,
            },
        }
    }

    pub fn rank_candidates(
        &self,
        mut candidates: Vec<NormalizedCandidate>,
        limit: usize,
    ) -> Vec<NormalizedCandidate> {
        for cand in &mut candidates {
            if cand.scoring.is_none() {
                cand.scoring = Some(self.score_candidate(cand));
            }
        }

        candidates.sort_by(|a, b| {
            let sa = a.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
            let sb = b.scoring.as_ref().map(|s| s.total_score).unwrap_or(0.0);
            match sb.partial_cmp(&sa) {
                Some(std::cmp::Ordering::Equal) | None => a.candidate_id.cmp(&b.candidate_id),
                Some(ord) => ord,
            }
        });

        candidates.truncate(limit);
        candidates
    }

    pub fn score_production_readiness(&self, candidate: &NormalizedCandidate) -> ProductionReadinessScoring {
        let verified_claims = candidate
            .claims
            .iter()
            .filter(|c| c.verification_status == "SOURCE_SUPPORTED" || c.verification_status == "INDEPENDENTLY_VERIFIED")
            .count();

        let independent_confirmations = candidate
            .claims
            .iter()
            .filter(|c| c.verification_status == "INDEPENDENTLY_VERIFIED")
            .count();

        let source_quality = match candidate.primary_source.as_ref() {
            Some(s) if s.fetch_status == "SUCCESS" && s.snapshot_sha256.is_some() => {
                if s.content_quality.as_deref() == Some("ARTICLE_CONTENT_VALID") {
                    1.0
                } else {
                    0.7
                }
            }
            _ => 0.0,
        };

        let visual_readiness = if !candidate.visual_evidence.is_empty() {
            let valid = candidate.visual_evidence.iter().all(|v| {
                matches!(
                    v.provenance_status
                        .as_deref()
                        .map(|s| s.trim().to_lowercase())
                        .as_deref(),
                    Some("verified") | Some("authoritative") | Some("licensed") | Some("approved")
                )
            });
            if valid { 1.0 } else { 0.5 }
        } else {
            0.0
        };

        let is_ready = matches!(
            candidate.research_readiness.as_ref().map(|r| r.status),
            Some(crate::domain::ReadinessStatus::VideoReady)
        );

        let total = (source_quality * 0.35)
            + ((verified_claims.min(5) as f64 / 5.0) * 0.25)
            + ((independent_confirmations.min(3) as f64 / 3.0) * 0.20)
            + (visual_readiness * 0.20);

        ProductionReadinessScoring {
            verified_claim_count: verified_claims,
            source_quality: (source_quality * 100.0).round() / 100.0,
            independent_confirmation_count: independent_confirmations,
            visual_asset_readiness: (visual_readiness * 100.0).round() / 100.0,
            production_readiness_score: (total * 10000.0).round() / 10000.0,
            is_video_production_ready: is_ready,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ProductionReadinessScoring {
    pub verified_claim_count: usize,
    pub source_quality: f64,
    pub independent_confirmation_count: usize,
    pub visual_asset_readiness: f64,
    pub production_readiness_score: f64,
    pub is_video_production_ready: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freshness_decay_curve() {
        let ref_epoch = 1793618000;
        let scorer = CandidateScorer::new(Some(ref_epoch));

        // 1 hour ago
        let score_1h = scorer.score_freshness(ref_epoch - 3600);
        assert_eq!(score_1h, 1.0);

        // 18 hours ago
        let score_18h = scorer.score_freshness(ref_epoch - 18 * 3600);
        assert!(score_18h > 0.1 && score_18h < 0.5);

        // 30 hours ago
        let score_30h = scorer.score_freshness(ref_epoch - 30 * 3600);
        assert_eq!(score_30h, 0.1);
    }

    #[test]
    fn test_publisher_authority_tiers() {
        let scorer = CandidateScorer::default();
        assert_eq!(scorer.score_publisher("Reuters"), 1.0);
        assert_eq!(scorer.score_publisher("The Verge"), 0.8);
        assert_eq!(scorer.score_publisher("Unknown"), 0.2);
    }

    #[test]
    fn test_observed_time_freshness_is_penalized() {
        let ref_epoch = 1793618000;
        let scorer = CandidateScorer::new(Some(ref_epoch));

        // When time is synthesized from feed observation or unknown, freshness is capped at 0.40
        let score_observed = scorer.score_freshness_with_kind(ref_epoch, Some("feed_observed"));
        assert_eq!(score_observed, 0.40);

        let score_unknown = scorer.score_freshness_with_kind(ref_epoch, Some("unknown"));
        assert_eq!(score_unknown, 0.40);

        // Genuine article publication timestamp within 1 hour receives full 1.0
        let score_published = scorer.score_freshness_with_kind(ref_epoch, Some("article_publication"));
        assert_eq!(score_published, 1.0);
    }
}
