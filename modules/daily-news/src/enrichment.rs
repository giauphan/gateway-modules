//! Candidate enrichment workflow orchestrator.
//!
//! Enforces Clean Architecture & DDD workflow gates:
//! Transitions candidates from DISCOVERED / NEEDS_SOURCE -> NEEDS_RESEARCH -> VIDEO_READY
//! with real snapshot capture, grounded claim verification, and persistent cache storage.

use crate::domain::{
    evaluate_readiness_full, ClaimEvidence, ContentOrigin, EvidenceRef, NormalizedCandidate,
    StoryboardEvidence, VisualEvidence,
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

        let snapshot_text = candidate
            .primary_source
            .as_ref()
            .and_then(|ps| ps.snapshot_ref.as_deref())
            .and_then(|ref_str| {
                self.capture
                    .as_ref()
                    .and_then(|cap| cap.read_snapshot_text(ref_str))
            });

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
        let secondary_reader = |hash: &str| -> Option<String> {
            self.capture
                .as_ref()
                .and_then(|cap| cap.read_snapshot_text(hash))
        };

        // Wire real asset byte resolver so visual provenance gate validates actual bytes
        let capture_ref = self.capture.clone();
        let asset_reader = move |sha: &str| -> Option<Vec<u8>> {
            capture_ref
                .as_ref()
                .and_then(|cap| cap.read_snapshot_bytes(sha))
        };

        // Wire real audit resolver so independent verification validates actual stored audit artifacts
        let capture_ref_audit = self.capture.clone();
        let audit_reader = move |sha: &str| -> Option<String> {
            capture_ref_audit
                .as_ref()
                .and_then(|cap| cap.read_snapshot_text(sha))
        };

        let readiness = evaluate_readiness_full(
            candidate.primary_source.as_ref(),
            snapshot_text.as_deref(),
            &candidate.claims,
            &candidate.visual_evidence,
            candidate.content_origin,
            Some(&secondary_reader),
            Some(&audit_reader),
            Some(&asset_reader),
        );
        let new_status = readiness.status.as_str().to_string();
        candidate.research_readiness = Some(readiness);

        // 8. Compute production readiness score
        let prod_scoring = self.scorer.score_production_readiness(&candidate);

        // 9. Persist updated candidate back into storage cache (propagating errors)
        self.storage.save_candidates(&[candidate.clone()])?;

        let transition = format!("{initial_status} -> {new_status}");

        Ok(EnrichedCandidateResponse {
            candidate,
            production_scoring: prod_scoring,
            transition,
        })
    }
}

const IMPLEMENTED_TOKENS: &[&str] = &[
    "demonstrated",
    "implemented",
    "deployed",
    "shipped",
    "released",
    "patched",
    "optimized",
    "built",
    "engineered",
    "integrated",
];

const PROPOSED_TOKENS: &[&str] = &[
    "proposes",
    "proposed",
    "proposal",
    "approach",
    "algorithm",
    "design",
    "concept",
    "rfc",
    "spec",
    "explores",
    "suggests",
    "hypothesizes",
];

const PROBLEM_TOKENS: &[&str] = &[
    "challenge",
    "problem",
    "bottleneck",
    "latency",
    "limitation",
    "vulnerability",
    "issue",
    "bug",
    "overhead",
    "flaw",
    "inefficiency",
    "slow",
    "fail",
    "difficult",
    "risk",
];

fn extract_raw_sentences(visible_text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = visible_text.char_indices().collect();
    let mut raw_sentences: Vec<&str> = Vec::new();
    let mut start = 0usize;

    for (idx, &(byte_idx, c)) in chars.iter().enumerate() {
        let next_is_digit = chars
            .get(idx + 1)
            .is_some_and(|(_, next_c)| next_c.is_ascii_digit());
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
    raw_sentences
}

fn has_empirical_metric(s: &str) -> bool {
    let lower = s.to_lowercase();
    if lower.contains('%') {
        return true;
    }
    const METRIC_WORDS: &[&str] = &[
        "speedup",
        "reduction",
        "latency",
        "throughput",
        "measured",
        "benchmark",
        "benchmarks",
        "overhead",
        "drop",
    ];
    lower.split(|c: char| !c.is_alphanumeric()).any(|w| {
        if let Some(rest) = w.strip_suffix('x') {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        } else if let Some(rest) = w.strip_suffix("ms") {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        } else {
            METRIC_WORDS.contains(&w)
        }
    })
}

fn is_negated_or_hypothetical(s: &str) -> bool {
    const NEGATION_PATTERNS: &[&str] = &[
        "not been benchmarked",
        "has not been benchmarked",
        "have not been benchmarked",
        "has not been tested",
        "have not been tested",
        "not yet benchmarked",
        "not yet been",
        "no benchmark",
        "unbenchmarked",
        "untested",
        "unproven",
        "yet to be",
        "remains to be",
        "future work",
        "hypothetically",
        "in theory",
        "without benchmarks",
        "no evidence",
        "not measured",
        "have not measured",
        "has not measured",
    ];
    let lower = s.to_lowercase();
    NEGATION_PATTERNS.iter().any(|pat| lower.contains(pat))
}

fn select_problem_sentence<'a>(raw_sentences: &[&'a str], excerpt: &'a str) -> &'a str {
    raw_sentences
        .iter()
        .find(|s| {
            let lower = s.to_lowercase();
            PROBLEM_TOKENS.iter().any(|tok| lower.contains(tok))
        })
        .or_else(|| raw_sentences.first())
        .copied()
        .unwrap_or(excerpt)
}

fn select_solution_sentence<'a>(raw_sentences: &[&'a str], problem: &'a str) -> &'a str {
    raw_sentences
        .iter()
        .find(|s| {
            let lower = s.to_lowercase();
            *s != &problem
                && (IMPLEMENTED_TOKENS.iter().any(|tok| lower.contains(tok))
                    || PROPOSED_TOKENS.iter().any(|tok| lower.contains(tok))
                    || lower.contains("solution")
                    || lower.contains("architecture"))
        })
        .or_else(|| raw_sentences.get(1))
        .copied()
        .unwrap_or(problem)
}

fn matches_outcome_criteria(s: &str) -> bool {
    !is_negated_or_hypothetical(s)
        && has_empirical_metric(s)
        && s.chars().any(|c| c.is_ascii_digit())
}

fn select_outcome_sentence<'a>(
    raw_sentences: &[&'a str],
    problem: &str,
    solution: &str,
) -> Option<&'a str> {
    raw_sentences
        .iter()
        .copied()
        .find(|s| *s != problem && *s != solution && matches_outcome_criteria(s))
        .or_else(|| raw_sentences.iter().copied().find(|s| matches_outcome_criteria(s)))
}

fn select_root_cause_sentence(raw_sentences: &[&str]) -> Option<String> {
    const ROOT_CAUSE_TOKENS: &[&str] = &[
        "root cause",
        "due to",
        "caused by",
        "flaw in",
        "bottleneck in",
        "vulnerability in",
    ];
    const ROOT_CAUSE_NEGATIONS: &[&str] = &[
        "not due to",
        "not caused by",
        "not the root cause",
        "unrelated to",
        "not a vulnerability",
    ];

    raw_sentences
        .iter()
        .copied()
        .find(|s| {
            let lower = s.to_lowercase();
            ROOT_CAUSE_TOKENS.iter().any(|tok| lower.contains(tok))
                && !ROOT_CAUSE_NEGATIONS.iter().any(|neg| lower.contains(neg))
        })
        .map(str::to_string)
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
    let raw_sentences = extract_raw_sentences(&visible_text);

    let problem_sentence = select_problem_sentence(&raw_sentences, excerpt);
    let solution_sentence = select_solution_sentence(&raw_sentences, problem_sentence);
    let outcome_sentence = select_outcome_sentence(&raw_sentences, problem_sentence, solution_sentence);

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

    // 2. Solution / Architecture claim with semantic modality distinction
    if solution_sentence != problem_sentence {
        let sol_lower = solution_sentence.to_lowercase();
        let is_implemented = IMPLEMENTED_TOKENS.iter().any(|tok| sol_lower.contains(tok))
            && !PROPOSED_TOKENS.iter().any(|tok| sol_lower.contains(tok));

        let statement = if is_implemented {
            format!("Implemented architectural remedy: {solution_sentence}")
        } else {
            format!("Proposed architectural approach: {solution_sentence}")
        };

        claims.push(ClaimEvidence {
            claim_id: format!("{candidate_id}_claim_solution"),
            statement,
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

    // 3. Measured Outcome / Benchmark claim (only if empirical numbers were observed)
    if let Some(outcome) = outcome_sentence {
        claims.push(ClaimEvidence {
            claim_id: format!("{candidate_id}_claim_outcome"),
            statement: format!("Observed empirical outcome: {outcome}"),
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

    let root_cause_sentence = select_root_cause_sentence(&raw_sentences);

    let remedy = if solution_sentence != problem_sentence
        && !is_negated_or_hypothetical(solution_sentence)
    {
        Some(solution_sentence.to_string())
    } else {
        None
    };

    let storyboard = StoryboardEvidence {
        hook: Some(format!("How {title} addresses core technical challenges")),
        viewer_question: Some(format!("What technical challenge did {title} address?")),
        symptom: Some(problem_sentence.to_string()),
        root_cause: root_cause_sentence,
        remedy,
        evidence_excerpt: Some(excerpt.to_string()),
        source_snapshot: snapshot_ref.map(str::to_string),
        snapshot_sha256: Some(sha.to_string()),
    };

    (claims, storyboard)
}
