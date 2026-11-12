//! Regression tests: discovery re-ingestion must never destroy captured research.

use daily_news::domain::{
    CandidateScoring, ClaimEvidence, EvidenceRef, NormalizedCandidate, PrimarySourceEvidence,
    ReadinessStatus, ResearchEvidenceFields, ResearchReadiness, ScoringAxes, VerificationRecord,
};
use daily_news::storage::{merge_candidate_entry, FileStorageAdapter};
use daily_news::StoragePort;

const PRIMARY_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn verified_readiness() -> ResearchReadiness {
    ResearchReadiness {
        status: ReadinessStatus::NeedsVerification,
        reasons: vec!["Awaiting independent verification".to_string()],
        missing_fields: vec!["independent_fact_check".to_string()],
        contract_version: "2.0".to_string(),
    }
}

fn enriched_candidate() -> NormalizedCandidate {
    NormalizedCandidate {
        candidate_id: "cand_merge_regression".to_string(),
        canonical_url: "https://example.com/article".to_string(),
        original_url: "https://example.com/article".to_string(),
        url: "https://example.com/article".to_string(),
        source_url: "https://example.com/article".to_string(),
        title: "Captured Article".to_string(),
        content_hash: "content_hash_v1".to_string(),
        body: Some("Captured primary source body excerpt".to_string()),
        description: Some("Captured description".to_string()),
        summary: Some("Captured summary".to_string()),
        research: ResearchEvidenceFields {
            primary_source: Some(PrimarySourceEvidence {
                url: "https://example.com/article".to_string(),
                fetch_status: "SUCCESS".to_string(),
                snapshot_ref: Some(format!(".state/evidence/{PRIMARY_SHA}.txt")),
                snapshot_sha256: Some(PRIMARY_SHA.to_string()),
                excerpt: Some("Captured primary source body excerpt".to_string()),
                ..Default::default()
            }),
            claims: vec![ClaimEvidence {
                claim_id: "claim_1".to_string(),
                statement: "Captured claim".to_string(),
                evidence_refs: vec![EvidenceRef {
                    source_ref: PRIMARY_SHA.to_string(),
                    excerpt: "Captured primary source body excerpt".to_string(),
                    locator: Some("section:1".to_string()),
                }],
                verification_status: "SOURCE_SUPPORTED".to_string(),
                reviewer: None,
                verification_record: None,
            }],
            research_readiness: Some(verified_readiness()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn headline_only_incoming(content_hash: &str) -> NormalizedCandidate {
    NormalizedCandidate {
        candidate_id: "cand_merge_regression".to_string(),
        canonical_url: "https://example.com/article".to_string(),
        original_url: "https://example.com/article".to_string(),
        url: "https://example.com/article".to_string(),
        source_url: "https://example.com/article".to_string(),
        title: "Captured Article".to_string(),
        content_hash: content_hash.to_string(),
        body: None,
        description: None,
        summary: None,
        research: ResearchEvidenceFields {
            primary_source: Some(PrimarySourceEvidence {
                url: "https://example.com/article".to_string(),
                fetch_status: "NOT_FETCHED".to_string(),
                ..Default::default()
            }),
            research_readiness: Some(ResearchReadiness {
                status: ReadinessStatus::NeedsSource,
                reasons: vec!["Primary article has not been captured or verified".to_string()],
                missing_fields: vec!["primary_source".to_string()],
                contract_version: "2.0".to_string(),
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn test_not_fetched_placeholder_does_not_overwrite_captured_research() {
    let mut existing = enriched_candidate();
    let incoming = headline_only_incoming("content_hash_v1");

    merge_candidate_entry(&mut existing, &incoming);

    let readiness = existing
        .research
        .research_readiness
        .as_ref()
        .expect("readiness must be preserved");
    assert_eq!(readiness.status, ReadinessStatus::NeedsVerification);
    assert_eq!(
        existing.body.as_deref(),
        Some("Captured primary source body excerpt")
    );
    assert_eq!(existing.description.as_deref(), Some("Captured description"));
    assert_eq!(existing.summary.as_deref(), Some("Captured summary"));
    assert_eq!(existing.research.claims.len(), 1);
    assert_eq!(
        existing
            .research
            .primary_source
            .as_ref()
            .map(|ps| ps.fetch_status.as_str()),
        Some("SUCCESS")
    );
}

#[test]
fn test_content_revision_preserves_body_but_requires_revalidation() {
    let mut existing = enriched_candidate();
    let mut incoming = headline_only_incoming("content_hash_v2");
    incoming.title = "Captured Article (edited)".to_string();

    merge_candidate_entry(&mut existing, &incoming);

    let readiness = existing
        .research
        .research_readiness
        .as_ref()
        .expect("readiness must be set");
    assert_eq!(readiness.status, ReadinessStatus::NeedsRevalidation);
    assert_eq!(
        existing.body.as_deref(),
        Some("Captured primary source body excerpt")
    );
    assert_eq!(existing.description.as_deref(), Some("Captured description"));
    assert_eq!(existing.summary.as_deref(), Some("Captured summary"));
}

#[test]
fn test_incoming_claims_merge_without_replacing_captured_source() {
    let mut existing = enriched_candidate();
    let mut incoming = headline_only_incoming("content_hash_v1");
    incoming.research.claims = vec![ClaimEvidence {
        claim_id: "claim_2".to_string(),
        statement: "Newly supplied claim".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: PRIMARY_SHA.to_string(),
            excerpt: "Newly supplied claim".to_string(),
            locator: Some("section:2".to_string()),
        }],
        verification_status: "SOURCE_SUPPORTED".to_string(),
        reviewer: None,
        verification_record: None,
    }];

    merge_candidate_entry(&mut existing, &incoming);

    assert_eq!(
        existing
            .research
            .primary_source
            .as_ref()
            .map(|ps| ps.fetch_status.as_str()),
        Some("SUCCESS")
    );
    assert_eq!(existing.research.claims.len(), 2);
    assert_eq!(
        existing.body.as_deref(),
        Some("Captured primary source body excerpt")
    );
    assert_eq!(
        existing
            .research
            .research_readiness
            .as_ref()
            .map(|r| r.status),
        Some(ReadinessStatus::NeedsRevalidation)
    );
}

#[test]
fn test_unchanged_claim_preserves_review_metadata() {
    let mut existing = enriched_candidate();
    existing.research.claims[0].reviewer = Some("auditor".to_string());
    existing.research.claims[0].verification_record = Some(VerificationRecord {
        record_id: "rec_1".to_string(),
        verified_at: "2026-11-10T00:00:00Z".to_string(),
        reviewer_id: "auditor".to_string(),
        organization: "Example Auditor".to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: None,
        secondary_source_ref: None,
    });
    let reviewed_claim = existing.research.claims[0].clone();

    let mut incoming = headline_only_incoming("content_hash_v1");
    incoming.research.claims = vec![ClaimEvidence {
        reviewer: None,
        verification_record: None,
        ..reviewed_claim
    }];

    merge_candidate_entry(&mut existing, &incoming);

    assert_eq!(existing.research.claims.len(), 1);
    assert_eq!(
        existing.research.claims[0].reviewer.as_deref(),
        Some("auditor")
    );
    assert_eq!(
        existing.research.claims[0]
            .verification_record
            .as_ref()
            .map(|r| r.record_id.as_str()),
        Some("rec_1")
    );
}

#[test]
fn test_reviewed_claims_survive_same_snapshot_recapture() {
    let mut existing = enriched_candidate();
    existing.research.claims[0].reviewer = Some("auditor".to_string());
    existing.research.claims[0].verification_status = "INDEPENDENTLY_VERIFIED".to_string();
    existing.research.claims[0].verification_record = Some(VerificationRecord {
        record_id: "rec_1".to_string(),
        verified_at: "2026-11-10T00:00:00Z".to_string(),
        reviewer_id: "auditor".to_string(),
        organization: "Example Auditor".to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: None,
        secondary_source_ref: None,
    });

    let mut incoming = enriched_candidate();
    incoming.research.claims[0].reviewer = None;
    incoming.research.claims[0].verification_record = None;
    incoming.research.claims[0].verification_status = "SOURCE_SUPPORTED".to_string();

    merge_candidate_entry(&mut existing, &incoming);

    assert_eq!(
        existing.research.claims[0].reviewer.as_deref(),
        Some("auditor")
    );
    assert_eq!(
        existing.research.claims[0].verification_status,
        "INDEPENDENTLY_VERIFIED"
    );
    assert_eq!(
        existing.research.claims[0]
            .verification_record
            .as_ref()
            .map(|r| r.record_id.as_str()),
        Some("rec_1")
    );
}

#[test]
fn test_snapshot_change_does_not_restore_prior_verification() {
    let mut existing = enriched_candidate();
    existing.research.claims[0].reviewer = Some("auditor".to_string());
    existing.research.claims[0].verification_status = "INDEPENDENTLY_VERIFIED".to_string();
    existing.research.claims[0].verification_record = Some(VerificationRecord {
        record_id: "rec_1".to_string(),
        verified_at: "2026-11-10T00:00:00Z".to_string(),
        reviewer_id: "auditor".to_string(),
        organization: "Example Auditor".to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: None,
        secondary_source_ref: None,
    });
    existing.independent_fact_check = Some(serde_json::json!({ "status": "verified" }));
    existing.evidence.symptom = Some("Old symptom".to_string());
    existing.evidence.snapshot_sha256 = Some(PRIMARY_SHA.to_string());

    let mut incoming = enriched_candidate();
    incoming
        .research
        .primary_source
        .as_mut()
        .expect("captured source")
        .snapshot_sha256 = Some("b".repeat(64));
    incoming.research.claims[0].reviewer = None;
    incoming.research.claims[0].verification_record = None;
    incoming.research.claims[0].verification_status = "SOURCE_SUPPORTED".to_string();
    incoming.independent_fact_check = None;
    incoming.evidence = Default::default();

    merge_candidate_entry(&mut existing, &incoming);

    assert!(existing.independent_fact_check.is_none());
    assert_eq!(existing.evidence.symptom, None);
    assert_eq!(
        existing.research.claims[0].verification_status,
        "SOURCE_SUPPORTED"
    );
    assert_eq!(existing.research.claims[0].reviewer, None);
}

#[test]
fn test_same_snapshot_recapture_preserves_readiness_when_incoming_missing() {
    let mut existing = enriched_candidate();
    let mut incoming = enriched_candidate();
    incoming.research.research_readiness = None;

    merge_candidate_entry(&mut existing, &incoming);

    assert_eq!(
        existing
            .research
            .research_readiness
            .as_ref()
            .map(|r| r.status),
        Some(ReadinessStatus::NeedsVerification)
    );
}

#[test]
fn test_same_snapshot_recapture_preserves_scoring() {
    let mut existing = enriched_candidate();
    existing.scoring = Some(CandidateScoring {
        total_score: 0.91,
        axes: ScoringAxes {
            freshness: 0.9,
            publisher_authority: 0.9,
            fact_density: 0.9,
            visual_potential: 0.9,
            audience_interest: 0.9,
            feasibility: 0.9,
        },
    });
    let mut incoming = enriched_candidate();
    incoming.scoring = None;

    merge_candidate_entry(&mut existing, &incoming);

    assert!(existing.scoring.is_some());
}

#[test]
fn test_partial_review_update_does_not_mix_revisions() {
    let mut existing = enriched_candidate();
    existing.research.claims[0].reviewer = Some("reviewer_a".to_string());
    existing.research.claims[0].verification_status = "INDEPENDENTLY_VERIFIED".to_string();
    existing.research.claims[0].verification_record = Some(VerificationRecord {
        record_id: "rec_a".to_string(),
        verified_at: "2026-11-10T00:00:00Z".to_string(),
        reviewer_id: "reviewer_a".to_string(),
        organization: "Example Auditor".to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: None,
        secondary_source_ref: None,
    });

    let mut incoming = enriched_candidate();
    incoming.research.claims[0].reviewer = Some("reviewer_b".to_string());
    incoming.research.claims[0].verification_record = None;
    incoming.research.claims[0].verification_status = "SOURCE_SUPPORTED".to_string();

    merge_candidate_entry(&mut existing, &incoming);

    assert_eq!(
        existing.research.claims[0].reviewer.as_deref(),
        Some("reviewer_b")
    );
    assert!(existing.research.claims[0].verification_record.is_none());
    assert_eq!(
        existing.research.claims[0].verification_status,
        "SOURCE_SUPPORTED"
    );
}

#[test]
fn test_persisted_enrichment_survives_reingestion() {
    let tmp = std::env::temp_dir().join(format!(
        "test_dn_merge_preserve_{}",
        uuid::Uuid::new_v4()
    ));
    let storage = FileStorageAdapter::new(&tmp);

    storage
        .save_candidates(&[enriched_candidate()])
        .expect("enriched candidate must persist");

    storage
        .save_candidates(&[headline_only_incoming("content_hash_v1")])
        .expect("re-ingested candidate must persist");

    let reloaded = FileStorageAdapter::new(&tmp);
    let candidate = reloaded
        .get_candidate("cand_merge_regression")
        .expect("candidate must survive restart");

    let readiness = candidate
        .research
        .research_readiness
        .as_ref()
        .expect("readiness must survive re-ingestion");
    assert_eq!(readiness.status, ReadinessStatus::NeedsVerification);
    assert_eq!(
        candidate.body.as_deref(),
        Some("Captured primary source body excerpt")
    );
    assert_eq!(candidate.research.claims.len(), 1);
    assert_eq!(
        candidate
            .research
            .primary_source
            .as_ref()
            .map(|ps| ps.fetch_status.as_str()),
        Some("SUCCESS")
    );

    let _ = std::fs::remove_dir_all(&tmp);
}