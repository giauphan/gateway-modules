//! Candidate enrichment workflow orchestrator.
//!
//! Enforces Clean Architecture & DDD workflow gates:
//! Transitions candidates from DISCOVERED / NEEDS_SOURCE -> NEEDS_RESEARCH -> VIDEO_READY
//! with real snapshot capture, grounded claim verification, and persistent cache storage.

use crate::domain::{
    evaluate_readiness_with_snapshot_text, ClaimEvidence, ContentOrigin, EvidenceRef,
    NormalizedCandidate, VisualEvidence,
};
use crate::ports::{SourceCapturePort, StoragePort};
use crate::scoring::{CandidateScorer, ProductionReadinessScoring};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedCandidateResponse {
    pub candidate: NormalizedCandidate,
    pub production_scoring: ProductionReadinessScoring,
    pub transition: String,
}

pub struct CandidateEnrichmentWorkflow {
    storage: Arc<dyn StoragePort>,
    capture: Option<Arc<dyn SourceCapturePort>>,
    scorer: CandidateScorer,
}

impl CandidateEnrichmentWorkflow {
    pub fn new(
        storage: Arc<dyn StoragePort>,
        capture: Option<Arc<dyn SourceCapturePort>>,
        scorer: CandidateScorer,
    ) -> Self {
        Self {
            storage,
            capture,
            scorer,
        }
    }

    pub fn enrich_candidate(
        &self,
        candidate_id: &str,
        override_url: Option<&str>,
        claims: Option<Vec<ClaimEvidence>>,
        visuals: Option<Vec<VisualEvidence>>,
    ) -> Result<EnrichedCandidateResponse, String> {
        let capturer = self
            .capture
            .as_ref()
            .ok_or_else(|| "Source capture adapter is not configured".to_string())?;

        // 1. Locate candidate from persistent cache
        let cached = self.storage.get_cached_candidates();
        let mut candidate = cached
            .into_iter()
            .find(|c| c.candidate_id == candidate_id)
            .ok_or_else(|| format!("Candidate not found for candidate_id: {candidate_id}"))?;

        let initial_status = candidate
            .research_readiness
            .as_ref()
            .map(|r| r.status.as_str())
            .unwrap_or("DISCOVERED")
            .to_string();

        // 2. Determine target capture URL
        let target_url = override_url
            .filter(|u| !u.trim().is_empty())
            .unwrap_or(&candidate.canonical_url);

        // 3. Execute primary source capture with SSRF defense & content quality gates
        let evidence = capturer.capture_source(target_url)?;

        let has_valid_capture = evidence.fetch_status == "SUCCESS"
            && evidence.snapshot_sha256.is_some()
            && evidence.content_quality.as_deref() == Some("ARTICLE_CONTENT_VALID");

        // 4. Attach primary source evidence (preserve valid prior capture if re-capture failed)
        if has_valid_capture || candidate.primary_source.is_none() {
            candidate.primary_source = Some(evidence.clone());
        }

        if has_valid_capture {
            if candidate.content_origin == ContentOrigin::HeadlineOnly
                || candidate.content_origin == ContentOrigin::Unknown
            {
                candidate.content_origin = ContentOrigin::Article;
            }

            let is_empty_body = candidate.body.as_deref().unwrap_or("").trim().is_empty();
            if override_url.is_some() || is_empty_body {
                candidate.body = evidence.excerpt.clone();
            }

            // 5. Update or ground claims
            if let Some(user_claims) = claims {
                candidate.claims = user_claims;
            } else if candidate.claims.is_empty() {
                if let (Some(ref sha), Some(ref exc)) =
                    (&evidence.snapshot_sha256, &evidence.excerpt)
                {
                    if exc.len() >= 10 {
                        let sample_excerpt = exc.chars().take(150).collect::<String>();
                        candidate.claims = vec![ClaimEvidence {
                            claim_id: format!("{}_claim_1", candidate.candidate_id),
                            statement: sample_excerpt.clone(),
                            evidence_refs: vec![EvidenceRef {
                                source_ref: sha.clone(),
                                excerpt: sample_excerpt,
                                locator: Some("paragraph:1".to_string()),
                            }],
                            verification_status: "SOURCE_SUPPORTED".to_string(),
                            reviewer: None,
                        }];
                    }
                }
            }

            // 6. Update visual evidence if provided
            if let Some(user_visuals) = visuals {
                candidate.visual_evidence = user_visuals;
            }
        }

        // 7. Evaluate new research readiness status using snapshot verification
        let snapshot_text = candidate
            .primary_source
            .as_ref()
            .and_then(|ps| ps.snapshot_ref.as_deref())
            .and_then(|ref_str| {
                self.capture
                    .as_ref()
                    .and_then(|cap| cap.read_snapshot_text(ref_str))
            });

        let readiness = evaluate_readiness_with_snapshot_text(
            candidate.primary_source.as_ref(),
            snapshot_text.as_deref(),
            &candidate.claims,
            &candidate.visual_evidence,
            candidate.content_origin,
        );
        let new_status = readiness.status.as_str().to_string();
        candidate.research_readiness = Some(readiness);

        // 8. Compute production readiness score
        let prod_scoring = self.scorer.score_production_readiness(&candidate);

        // 9. Persist updated candidate back into storage cache
        self.storage.cache_candidates(&[candidate.clone()]);

        let transition = format!("{initial_status} -> {new_status}");

        Ok(EnrichedCandidateResponse {
            candidate,
            production_scoring: prod_scoring,
            transition,
        })
    }
}
