//! Deterministic 6-axis candidate scoring and ranking.

use crate::domain::{CandidateScoring, NormalizedCandidate, ScoringAxes};

pub struct CandidateScorer {
    pub weights: (f64, f64, f64, f64, f64, f64),
}

impl Default for CandidateScorer {
    fn default() -> Self {
        Self {
            weights: (0.25, 0.20, 0.15, 0.15, 0.15, 0.10),
        }
    }
}

impl CandidateScorer {
    pub fn score_candidate(&self, candidate: &NormalizedCandidate) -> CandidateScoring {
        let (w_f, w_a, w_d, w_v, w_i, w_fe) = self.weights;

        let freshness = 0.95; // Fresh items within window
        let pub_authority = match candidate.normalized_publisher.to_lowercase().as_str() {
            "anthropic" | "openai" | "google" | "deepmind" => 1.0,
            "github" | "arxiv" | "nature" | "science" => 0.85,
            "hacker news" | "techcrunch" | "the verge" => 0.70,
            _ => 0.50,
        };
        let fact_density = if candidate.body.as_deref().unwrap_or("").len() > 200 {
            0.8
        } else {
            0.5
        };
        let visual_potential = if candidate.discussion_url.is_some() {
            0.6
        } else {
            0.4
        };
        let audience_interest = 0.8;
        let feasibility = 1.0;

        let total = (freshness * w_f)
            + (pub_authority * w_a)
            + (fact_density * w_d)
            + (visual_potential * w_v)
            + (audience_interest * w_i)
            + (feasibility * w_fe);

        CandidateScoring {
            total_score: (total * 100.0).round() / 100.0,
            axes: ScoringAxes {
                freshness,
                publisher_authority: pub_authority,
                fact_density,
                visual_potential,
                audience_interest,
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
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(limit);
        candidates
    }
}
