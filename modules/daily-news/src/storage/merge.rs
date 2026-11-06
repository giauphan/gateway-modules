//! Candidate merging, revision preservation, and cache sizing rules.

use crate::domain::{NormalizedCandidate, StoryboardEvidence};

fn restore_preserved_verification(
    target: &mut NormalizedCandidate,
    preserved_fact_check: Option<serde_json::Value>,
    preserved_evidence: StoryboardEvidence,
) {
    if target.independent_fact_check.is_none() {
        target.independent_fact_check = preserved_fact_check;
    }
    if target.evidence.snapshot_sha256.is_none() {
        target.evidence = preserved_evidence;
    }
}

fn merge_candidate_with_evidence(
    existing: &mut NormalizedCandidate,
    incoming: &NormalizedCandidate,
) {
    let preserved_fact_check = existing.independent_fact_check.clone();
    let preserved_evidence = existing.evidence.clone();
    *existing = incoming.clone();
    restore_preserved_verification(existing, preserved_fact_check, preserved_evidence);
}

fn merge_candidate_preserve_research(
    existing: &mut NormalizedCandidate,
    incoming: &NormalizedCandidate,
) {
    let preserved_research = existing.research.clone();
    let preserved_fact_check = existing.independent_fact_check.clone();
    let preserved_evidence = existing.evidence.clone();
    let preserved_scoring = existing.scoring.clone();

    *existing = incoming.clone();
    existing.research = preserved_research;
    restore_preserved_verification(existing, preserved_fact_check, preserved_evidence);
    if existing.scoring.is_none() {
        existing.scoring = preserved_scoring;
    }
}

pub fn merge_candidate_entry(existing: &mut NormalizedCandidate, incoming: &NormalizedCandidate) {
    let content_changed = existing.content_hash != incoming.content_hash;

    if content_changed {
        // Content body changed across revisions! Stale research and verification cannot be preserved.
        let mut updated = incoming.clone();
        let incoming_has_readiness = incoming
            .research
            .research_readiness
            .as_ref()
            .is_some_and(|r| {
                r.status == crate::domain::ReadinessStatus::VideoReady
                    || r.status == crate::domain::ReadinessStatus::MockVerified
            });

        if !incoming_has_readiness {
            updated.research_readiness = Some(crate::domain::ResearchReadiness {
                status: crate::domain::ReadinessStatus::NeedsRevalidation,
                reasons: vec![format!(
                    "Content body modified across ingestion revisions (old content_hash: {}, new: {}); stale research invalidated",
                    existing.content_hash, incoming.content_hash
                )],
                missing_fields: vec!["content_revalidation".to_string()],
                contract_version: "2.0".to_string(),
            });
        }
        *existing = updated;
        return;
    }

    let incoming_has_evidence =
        incoming.research.primary_source.is_some() || !incoming.research.claims.is_empty();

    if incoming_has_evidence || existing.research.primary_source.is_none() {
        merge_candidate_with_evidence(existing, incoming);
    } else {
        merge_candidate_preserve_research(existing, incoming);
    }
}

pub fn trim_candidate_cache(guard: &mut Vec<NormalizedCandidate>, max_size: usize) {
    if guard.len() > max_size {
        let excess = guard.len() - max_size;
        guard.drain(0..excess);
    }
}
