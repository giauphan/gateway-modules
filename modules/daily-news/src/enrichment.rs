//! Candidate enrichment workflow orchestrator.
//!
//! Enforces Clean Architecture & DDD workflow gates:
//! Transitions candidates from DISCOVERED / NEEDS_SOURCE -> NEEDS_RESEARCH -> VIDEO_READY
//! with real snapshot capture, grounded claim verification, and persistent cache storage.

use crate::domain::{
    evaluate_readiness_with_snapshot_text, ClaimEvidence, ContentOrigin, EvidenceRef,
    NormalizedCandidate, StoryboardEvidence, VisualEvidence,
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

            // Read snapshot text early so research package and readiness can use it
            let snapshot_text = candidate
                .primary_source
                .as_ref()
                .and_then(|ps| ps.snapshot_ref.as_deref())
                .and_then(|ref_str| {
                    self.capture
                        .as_ref()
                        .and_then(|cap| cap.read_snapshot_text(ref_str))
                });

            // 5. Build substantive research package (problem, solution, outcome) or attach provided claims
            if let Some(user_claims) = claims {
                candidate.claims = user_claims;
            } else if candidate.claims.is_empty() {
                if let (Some(ref sha), Some(ref exc)) =
                    (&evidence.snapshot_sha256, &evidence.excerpt)
                {
                    if exc.len() >= 10 {
                        let text_ref = snapshot_text.as_deref().unwrap_or(exc);
                        let (claims_pkg, storyboard) = build_substantive_research_package(
                            &candidate.candidate_id,
                            sha,
                            exc,
                            &candidate.title,
                            text_ref,
                            evidence.snapshot_ref.as_deref(),
                        );
                        candidate.claims = claims_pkg;
                        candidate.evidence = storyboard;
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

pub fn build_substantive_research_package(
    candidate_id: &str,
    sha: &str,
    excerpt: &str,
    title: &str,
    snapshot_text: &str,
    snapshot_ref: Option<&str>,
) -> (Vec<ClaimEvidence>, StoryboardEvidence) {
    let visible_text = crate::source_capture::extract_main_text(snapshot_text);
    let chars: Vec<(usize, char)> = visible_text.char_indices().collect();
    let mut raw_sentences: Vec<&str> = Vec::new();
    let mut start = 0usize;

    for (idx, &(byte_idx, c)) in chars.iter().enumerate() {
        let next_is_digit = chars.get(idx + 1).is_some_and(|(_, next_c)| next_c.is_ascii_digit());
        if matches!(c, '!' | '?' | '\n') || (c == '.' && !next_is_digit) {
            let s = visible_text[start..byte_idx].trim();
            if s.len() >= 15 {
                raw_sentences.push(s);
            }
            start = byte_idx + c.len_utf8();
        }
    }
    let tail = visible_text[start..].trim();
    if tail.len() >= 15 {
        raw_sentences.push(tail);
    }

    let problem_tokens = &[
        "challenge", "problem", "bottleneck", "latency", "limitation",
        "vulnerability", "issue", "bug", "overhead", "flaw", "inefficiency",
        "slow", "fail", "difficult", "risk",
    ];

    let solution_tokens = &[
        "demonstrated", "introduced", "implemented", "optimized", "built",
        "designed", "proposes", "solution", "algorithm", "architecture",
        "framework", "release", "patch", "uses", "developed", "approach",
    ];

    let outcome_tokens = &[
        "reduction", "improvement", "speedup", "benchmark", "measured",
        "throughput", "faster", "drop", "accuracy", "gain", "results", "%", "4x", "2x",
    ];

    let problem_sentence = raw_sentences
        .iter()
        .find(|s| {
            let lower = s.to_lowercase();
            problem_tokens.iter().any(|tok| lower.contains(tok))
        })
        .or_else(|| raw_sentences.first())
        .copied()
        .unwrap_or(excerpt);

    let solution_sentence = raw_sentences
        .iter()
        .find(|s| {
            let lower = s.to_lowercase();
            *s != &problem_sentence && solution_tokens.iter().any(|tok| lower.contains(tok))
        })
        .or_else(|| raw_sentences.get(1))
        .copied()
        .unwrap_or(problem_sentence);

    let outcome_sentence = raw_sentences
        .iter()
        .find(|s| {
            let lower = s.to_lowercase();
            *s != &problem_sentence && *s != &solution_sentence && outcome_tokens.iter().any(|tok| lower.contains(tok))
        })
        .or_else(|| raw_sentences.iter().find(|s| {
            let lower = s.to_lowercase();
            outcome_tokens.iter().any(|tok| lower.contains(tok))
        }))
        .copied();

    let mut claims = Vec::new();

    // 1. Problem / Bottleneck claim
    claims.push(ClaimEvidence {
        claim_id: format!("{candidate_id}_claim_problem"),
        statement: format!("Technical problem identified: {problem_sentence}"),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha.to_string(),
            excerpt: problem_sentence.to_string(),
            locator: Some("section:problem".to_string()),
        }],
        verification_status: "SOURCE_SUPPORTED".to_string(),
        reviewer: None,
        verification_record: None,
    });

    // 2. Solution / Architecture claim
    if solution_sentence != problem_sentence {
        claims.push(ClaimEvidence {
            claim_id: format!("{candidate_id}_claim_solution"),
            statement: format!("Architectural remedy applied: {solution_sentence}"),
            evidence_refs: vec![EvidenceRef {
                source_ref: sha.to_string(),
                excerpt: solution_sentence.to_string(),
                locator: Some("section:solution".to_string()),
            }],
            verification_status: "SOURCE_SUPPORTED".to_string(),
            reviewer: None,
            verification_record: None,
        });
    }

    // 3. Measured Outcome / Benchmark claim
    if let Some(outcome) = outcome_sentence {
        claims.push(ClaimEvidence {
            claim_id: format!("{candidate_id}_claim_outcome"),
            statement: format!("Demonstrated empirical outcome: {outcome}"),
            evidence_refs: vec![EvidenceRef {
                source_ref: sha.to_string(),
                excerpt: outcome.to_string(),
                locator: Some("section:outcome".to_string()),
            }],
            verification_status: "SOURCE_SUPPORTED".to_string(),
            reviewer: None,
            verification_record: None,
        });
    }

    let storyboard = StoryboardEvidence {
        hook: Some(format!("How {title} addresses core technical bottlenecks")),
        viewer_question: Some(format!("What technical challenge did {title} solve and what were the verified results?")),
        symptom: Some(problem_sentence.to_string()),
        root_cause: Some(format!("Root architectural cause addressed by {title}")),
        remedy: Some(solution_sentence.to_string()),
        evidence_excerpt: Some(excerpt.to_string()),
        source_snapshot: snapshot_ref.map(str::to_string),
        snapshot_sha256: Some(sha.to_string()),
    };

    (claims, storyboard)
}
