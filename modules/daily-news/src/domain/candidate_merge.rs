//! Candidate revision merge policy.
//!
//! Pure domain rules governing how re-ingested feed revisions interact with
//! captured research evidence, claims, and readiness state. No I/O dependencies.

use crate::domain::{
    ClaimEvidence, NormalizedCandidate, PrimarySourceEvidence, StoryboardEvidence,
};

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(|s| s.trim().is_empty())
}

fn has_captured_primary_source(primary: Option<&PrimarySourceEvidence>) -> bool {
    primary.is_some_and(|s| {
        s.fetch_status == "SUCCESS"
            && s.snapshot_sha256.as_deref().is_some_and(|hash| {
                let trimmed = hash.trim();
                trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
            })
    })
}

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

fn restore_preserved_content(
    target: &mut NormalizedCandidate,
    preserved_body: Option<String>,
    preserved_description: Option<String>,
    preserved_summary: Option<String>,
) {
    if is_blank(target.body.as_deref()) {
        target.body = preserved_body;
    }
    if is_blank(target.description.as_deref()) {
        target.description = preserved_description;
    }
    if is_blank(target.summary.as_deref()) {
        target.summary = preserved_summary;
    }
}

fn merge_candidate_with_evidence(
    existing: &mut NormalizedCandidate,
    incoming: &NormalizedCandidate,
) {
    let preserved_fact_check = existing.independent_fact_check.clone();
    let preserved_evidence = existing.evidence.clone();
    let preserved_claims = existing.research.claims.clone();
    let preserved_readiness = existing.research_readiness.clone();
    let preserved_scoring = existing.scoring.clone();
    let same_content = existing.content_hash == incoming.content_hash;
    let same_snapshot = existing
        .research
        .primary_source
        .as_ref()
        .and_then(|ps| ps.snapshot_sha256.as_deref())
        == incoming
            .research
            .primary_source
            .as_ref()
            .and_then(|ps| ps.snapshot_sha256.as_deref());
    let incoming_is_captured =
        has_captured_primary_source(incoming.research.primary_source.as_ref());
    let claims_changed =
        !claim_sets_content_equal(&preserved_claims, &incoming.research.claims);
    let incoming_has_readiness = incoming
        .research
        .research_readiness
        .as_ref()
        .is_some_and(|r| {
            r.status == crate::domain::ReadinessStatus::VideoReady
                || r.status == crate::domain::ReadinessStatus::MockVerified
        });

    *existing = incoming.clone();
    if same_content && same_snapshot && incoming_is_captured {
        restore_claim_reviews(&mut existing.research.claims, &preserved_claims);
        if claims_changed && !incoming_has_readiness {
            existing.research_readiness = Some(crate::domain::ResearchReadiness {
                status: crate::domain::ReadinessStatus::NeedsRevalidation,
                reasons: vec![
                    "Captured claims changed across revisions; dependent readiness requires revalidation"
                        .to_string(),
                ],
                missing_fields: vec!["claim_revalidation".to_string()],
                contract_version: "2.0".to_string(),
            });
        } else if !claims_changed && existing.research_readiness.is_none() {
            existing.research_readiness = preserved_readiness;
        }
        if existing.scoring.is_none() {
            existing.scoring = preserved_scoring;
        }
    }
    if same_snapshot {
        restore_preserved_verification(existing, preserved_fact_check, preserved_evidence);
    }
}

fn merge_candidate_preserve_research(
    existing: &mut NormalizedCandidate,
    incoming: &NormalizedCandidate,
) {
    let preserved_research = existing.research.clone();
    let preserved_fact_check = existing.independent_fact_check.clone();
    let preserved_evidence = existing.evidence.clone();
    let preserved_scoring = existing.scoring.clone();
    let preserved_body = existing.body.clone();
    let preserved_description = existing.description.clone();
    let preserved_summary = existing.summary.clone();

    *existing = incoming.clone();
    existing.research = preserved_research;
    restore_preserved_verification(existing, preserved_fact_check, preserved_evidence);
    restore_preserved_content(
        existing,
        preserved_body,
        preserved_description,
        preserved_summary,
    );
    if existing.scoring.is_none() {
        existing.scoring = preserved_scoring;
    }
}

fn claim_content_matches(existing: &ClaimEvidence, incoming: &ClaimEvidence) -> bool {
    existing.claim_id == incoming.claim_id
        && existing.statement == incoming.statement
        && existing.evidence_refs == incoming.evidence_refs
}

fn claim_sets_content_equal(left: &[ClaimEvidence], right: &[ClaimEvidence]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .all(|claim| right.iter().any(|other| claim_content_matches(claim, other)))
}

fn restore_claim_reviews(target: &mut Vec<ClaimEvidence>, prior: &[ClaimEvidence]) {
    for old in prior {
        if let Some(slot) = target.iter_mut().find(|c| c.claim_id == old.claim_id) {
            if claim_content_matches(slot, old) {
                let incoming_has_review =
                    slot.reviewer.is_some() || slot.verification_record.is_some();
                if !incoming_has_review {
                    slot.reviewer = old.reviewer.clone();
                    slot.verification_record = old.verification_record.clone();
                    slot.verification_status = old.verification_status.clone();
                }
            }
        } else {
            target.push(old.clone());
        }
    }
}

fn merge_claims(existing: &mut Vec<ClaimEvidence>, incoming: &[ClaimEvidence]) -> bool {
    let mut changed = false;
    for claim in incoming {
        if let Some(slot) = existing.iter_mut().find(|c| c.claim_id == claim.claim_id) {
            let content_matches = claim_content_matches(slot, claim);
            if !content_matches {
                changed = true;
            }
            let mut merged = claim.clone();
            if content_matches {
                let incoming_has_review =
                    claim.reviewer.is_some() || claim.verification_record.is_some();
                if !incoming_has_review {
                    merged.reviewer = slot.reviewer.clone();
                    merged.verification_record = slot.verification_record.clone();
                    merged.verification_status = slot.verification_status.clone();
                }
            }
            *slot = merged;
        } else {
            existing.push(claim.clone());
            changed = true;
        }
    }
    changed
}

pub fn merge_candidate_entry(existing: &mut NormalizedCandidate, incoming: &NormalizedCandidate) {
    let content_changed = existing.content_hash != incoming.content_hash;

    if content_changed {
        // Content body changed across revisions: stale research must be revalidated, but
        // captured article content from the prior revision must not be discarded.
        let preserved_body = existing.body.clone();
        let preserved_description = existing.description.clone();
        let preserved_summary = existing.summary.clone();
        let mut updated = incoming.clone();
        restore_preserved_content(
            &mut updated,
            preserved_body,
            preserved_description,
            preserved_summary,
        );

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

    // Discovery adapters emit a NOT_FETCHED placeholder primary source for headline-only
    // items. That placeholder is not evidence and must never overwrite captured research.
    // Incoming claims are still merged without replacing an existing captured source.
    let incoming_has_captured_source =
        has_captured_primary_source(incoming.research.primary_source.as_ref());

    if incoming_has_captured_source || existing.research.primary_source.is_none() {
        merge_candidate_with_evidence(existing, incoming);
    } else {
        merge_candidate_preserve_research(existing, incoming);
        if merge_claims(&mut existing.research.claims, &incoming.research.claims) {
            existing.research_readiness = Some(crate::domain::ResearchReadiness {
                status: crate::domain::ReadinessStatus::NeedsRevalidation,
                reasons: vec![
                    "Claims changed from an uncaptured source; dependent readiness requires revalidation"
                        .to_string(),
                ],
                missing_fields: vec!["claim_revalidation".to_string()],
                contract_version: "2.0".to_string(),
            });
        }
    }
}