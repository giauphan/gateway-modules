use daily_news::domain::{
    compute_sha256, evaluate_readiness_with_snapshot_store, ClaimEvidence, ContentOrigin,
    EvidenceRef, PrimarySourceEvidence, ReadinessStatus, VerificationRecord, VisualEvidence,
};
use daily_news::enrichment::build_substantive_research_package;
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::FileStorageAdapter;
use daily_news::RawNewsItem;
use std::sync::Arc;

fn sample_visual(asset_ref: &str, kind: &str, source_url: &str) -> VisualEvidence {
    VisualEvidence {
        asset_ref: asset_ref.to_string(),
        kind: kind.to_string(),
        source_url: source_url.to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(compute_sha256("mock visual asset bytes")),
        license_evidence_ref: Some("https://example.com/license-terms".to_string()),
        verified_rights: Some(true),
    }
}

fn sample_accredited_record(org: &str) -> VerificationRecord {
    VerificationRecord {
        record_id: "vr_v110_001".to_string(),
        verified_at: "2026-11-04T05:00:00Z".to_string(),
        reviewer_id: "accredited_auditor_01".to_string(),
        organization: org.to_string(),
        method: "reproducible_benchmark".to_string(),
        audit_trail_ref: Some(compute_sha256("Official audit artifact from accredited organization")),
        secondary_source_ref: None,
    }
}

#[test]
fn test_independent_verification_reviewer_fallback_strictly_removed() {
    let text = "Primary benchmark content showing real speedup.";
    let sha = compute_sha256(text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/test".to_string(),
        snapshot_ref: Some(format!(".state/evidence/{sha}.txt")),
        snapshot_sha256: Some(sha.clone()),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Title".to_string()),
        published_at: None,
        excerpt: Some(text.to_string()),
        captured_at: None,
        extraction_method: None,
    };

    // Caller self-declares accredited reviewer name "Benchmarking Laboratory QA", but verification_record is None
    let spoofed_claim = ClaimEvidence {
        claim_id: "c_spoof".to_string(),
        statement: "Real speedup".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha.clone(),
            excerpt: "Primary benchmark content".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None, // No genuine VerificationRecord!
    };

    let visuals = vec![sample_visual("vis_1", "chart", "https://example.com/chart.png")];

    let readiness = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(text),
        &[spoofed_claim],
        &visuals,
        ContentOrigin::Article,
        None,
    );

    // CRITICAL: Reviewer name alone MUST NOT bypass independent verification gate!
    assert_ne!(
        readiness.status,
        ReadinessStatus::VideoReady,
        "Reviewer name without VerificationRecord must NEVER reach VIDEO_READY"
    );
    assert_eq!(readiness.status, ReadinessStatus::NeedsVerification);
    assert!(readiness.missing_fields.contains(&"independent_fact_check".to_string()));
}

#[test]
fn test_verification_record_missing_audit_trail_ref_fails_closed() {
    let mut rec = sample_accredited_record("Benchmarking Laboratory");
    rec.audit_trail_ref = None; // Missing audit trail SHA-256!

    let text = "Primary benchmark content showing real speedup.";
    let sha = compute_sha256(text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/test".to_string(),
        snapshot_ref: Some(format!(".state/evidence/{sha}.txt")),
        snapshot_sha256: Some(sha.clone()),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Title".to_string()),
        published_at: None,
        excerpt: Some(text.to_string()),
        captured_at: None,
        extraction_method: None,
    };

    let claim = ClaimEvidence {
        claim_id: "c_rec".to_string(),
        statement: "Real speedup".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha,
            excerpt: "Primary benchmark content".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(rec),
    };

    let visuals = vec![sample_visual("vis_1", "chart", "https://example.com/chart.png")];

    let readiness = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(text),
        &[claim],
        &visuals,
        ContentOrigin::Article,
        None,
    );

    assert_eq!(
        readiness.status,
        ReadinessStatus::NeedsVerification,
        "Verification record without 64-hex audit_trail_ref must fail closed"
    );
}

#[test]
fn test_secondary_evidence_snapshot_artifact_verification() {
    let primary_text = "Primary whitepaper describing the lock-free data structure.";
    let primary_sha = compute_sha256(primary_text);

    let sec_text = "Independent lab report: Reproduced 4x throughput under burst concurrency.";
    let sec_sha = compute_sha256(sec_text);

    let primary_source = PrimarySourceEvidence {
        url: "https://example.com/whitepaper".to_string(),
        snapshot_ref: Some(format!(".state/evidence/{primary_sha}.txt")),
        snapshot_sha256: Some(primary_sha.clone()),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Lock-free Data Structure".to_string()),
        published_at: None,
        excerpt: Some(primary_text.to_string()),
        captured_at: None,
        extraction_method: None,
    };

    let mut rec = sample_accredited_record("Benchmarking Laboratory");
    rec.secondary_source_ref = Some(sec_sha.clone());

    let claim = ClaimEvidence {
        claim_id: "c_sec".to_string(),
        statement: "Reproduced 4x throughput under burst concurrency".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sec_sha.clone(),
            excerpt: "Reproduced 4x throughput under burst concurrency.".to_string(),
            locator: Some("sec:lab_results".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(rec),
    };

    let visuals = vec![sample_visual("vis_1", "chart", "https://example.com/chart.png")];

    // Case 1: Secondary snapshot artifact does NOT exist in store -> fails closed with NeedsResearch!
    let r_missing_artifact = evaluate_readiness_with_snapshot_store(
        Some(&primary_source),
        Some(primary_text),
        std::slice::from_ref(&claim),
        &visuals,
        ContentOrigin::Article,
        None, // No secondary snapshot available
    );
    assert_eq!(
        r_missing_artifact.status,
        ReadinessStatus::NeedsResearch,
        "Citing secondary source without stored artifact must fail closed"
    );
    assert!(r_missing_artifact.missing_fields.contains(&"valid_evidence_refs".to_string()));

    // Case 2: Secondary snapshot artifact exists, but excerpt is not in it -> fails closed!
    let corrupted_sec_text = "Totally unrelated text that does not contain the cited excerpt.";
    let corrupted_resolver = |_hash: &str| Some(corrupted_sec_text.to_string());
    let r_corrupted = evaluate_readiness_with_snapshot_store(
        Some(&primary_source),
        Some(primary_text),
        std::slice::from_ref(&claim),
        &visuals,
        ContentOrigin::Article,
        Some(&corrupted_resolver),
    );
    assert_eq!(
        r_corrupted.status,
        ReadinessStatus::NeedsResearch,
        "Secondary snapshot hash mismatch or missing excerpt must fail closed"
    );

    // Case 3: Secondary snapshot artifact exists, SHA matches, and excerpt is grounded.
    // Under unified verification gate, without audit/asset resolvers, it holds at NeedsVerification!
    let valid_resolver = move |hash: &str| {
        if hash == sec_sha {
            Some(sec_text.to_string())
        } else {
            None
        }
    };
    let r_success = evaluate_readiness_with_snapshot_store(
        Some(&primary_source),
        Some(primary_text),
        &[claim],
        &visuals,
        ContentOrigin::Article,
        Some(&valid_resolver),
    );
    assert_ne!(
        r_success.status,
        ReadinessStatus::VideoReady,
        "Without audit/asset resolvers, candidates cannot bypass verification to VIDEO_READY"
    );
    assert_eq!(r_success.status, ReadinessStatus::NeedsVerification);
}

#[test]
fn test_visual_evidence_checksum_and_rights_enforcement() {
    let text = "Whitepaper content with valid benchmark data.";
    let sha = compute_sha256(text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/whitepaper".to_string(),
        snapshot_ref: Some(format!(".state/evidence/{sha}.txt")),
        snapshot_sha256: Some(sha.clone()),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Title".to_string()),
        published_at: None,
        excerpt: Some(text.to_string()),
        captured_at: None,
        extraction_method: None,
    };

    let claim = ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "Valid benchmark data".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha,
            excerpt: "valid benchmark data".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(sample_accredited_record("Benchmarking Laboratory")),
    };

    // 1. Missing checksum_sha256
    let mut vis_no_checksum = sample_visual("vis_1", "chart", "https://example.com/chart.png");
    vis_no_checksum.checksum_sha256 = None;
    let r1 = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(text),
        std::slice::from_ref(&claim),
        &[vis_no_checksum],
        ContentOrigin::Article,
        None,
    );
    assert_eq!(r1.status, ReadinessStatus::NeedsVisualEvidence);

    // 2. Missing license_evidence_ref
    let mut vis_no_lic_ref = sample_visual("vis_1", "chart", "https://example.com/chart.png");
    vis_no_lic_ref.license_evidence_ref = None;
    let r2 = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(text),
        std::slice::from_ref(&claim),
        &[vis_no_lic_ref],
        ContentOrigin::Article,
        None,
    );
    assert_eq!(r2.status, ReadinessStatus::NeedsVisualEvidence);

    // 3. verified_rights: false
    let mut vis_no_rights = sample_visual("vis_1", "chart", "https://example.com/chart.png");
    vis_no_rights.verified_rights = Some(false);
    let r3 = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(text),
        std::slice::from_ref(&claim),
        &[vis_no_rights],
        ContentOrigin::Article,
        None,
    );
    assert_eq!(r3.status, ReadinessStatus::NeedsVisualEvidence);

    // 4. Fully compliant visual evidence metadata without audit/asset resolvers holds at NeedsVerification
    let vis_valid = sample_visual("vis_1", "chart", "https://example.com/chart.png");
    let r4 = evaluate_readiness_with_snapshot_store(
        Some(&source),
        Some(text),
        &[claim],
        &[vis_valid],
        ContentOrigin::Article,
        None,
    );
    assert_ne!(
        r4.status,
        ReadinessStatus::VideoReady,
        "Cannot reach VIDEO_READY without audit and asset resolvers"
    );
    assert_eq!(r4.status, ReadinessStatus::NeedsVerification);
}

#[test]
fn test_storyboard_semantic_modality_proposed_vs_implemented_and_unasserted_outcomes() {
    // 1. Pure proposal text: must emit "Proposed architectural approach" and NO fabricated outcome or root cause
    let proposal_text = "This RFC proposes a new actor-based consensus approach for distributed databases. We explore a gossip-based communication protocol for node discovery.";
    let sha_prop = compute_sha256(proposal_text);

    let (claims_prop, storyboard_prop) = build_substantive_research_package(
        "cand_proposal",
        &sha_prop,
        "This RFC proposes a new actor-based consensus approach",
        "Actor Consensus RFC",
        proposal_text,
        Some(".state/evidence/proposal.txt"),
    );

    // Must NOT fabricate "Architectural remedy applied"
    assert!(
        claims_prop.iter().any(|c| c.statement.starts_with("Proposed architectural approach:")),
        "Proposal RFC must be labeled as Proposed architectural approach"
    );
    assert!(
        !claims_prop.iter().any(|c| c.statement.starts_with("Architectural remedy applied:")),
        "Must NOT assert applied remedy for a proposal"
    );
    // Must NOT fabricate outcome claim when no empirical measurements exist
    assert!(
        !claims_prop.iter().any(|c| c.statement.starts_with("Observed empirical outcome:")),
        "Must NOT emit outcome claim without empirical numbers"
    );
    // Must NOT hallucinate assumed root cause
    assert_eq!(storyboard_prop.root_cause, None, "Root cause must be None when unevidenced in text");

    // 2. Implemented engineering text with empirical numbers: emits Implemented and Observed empirical outcome
    let impl_text = "Identified severe queue backlog under spike traffic. We implemented an asynchronous ring buffer. Benchmarks demonstrate a 4x reduction in tail latency and 35% throughput gain.";
    let sha_impl = compute_sha256(impl_text);

    let (claims_impl, _storyboard_impl) = build_substantive_research_package(
        "cand_impl",
        &sha_impl,
        "Identified severe queue backlog under spike traffic.",
        "Ring Buffer Engine",
        impl_text,
        Some(".state/evidence/impl.txt"),
    );

    assert!(
        claims_impl.iter().any(|c| c.statement.starts_with("Implemented architectural remedy:")),
        "Demonstrated implementation must be labeled as Implemented architectural remedy"
    );
    assert!(
        claims_impl.iter().any(|c| c.statement.starts_with("Observed empirical outcome:")),
        "Empirical measurement must be labeled as Observed empirical outcome"
    );

    // 3. Substring false positive rejection: words like "platforms", "systems", "items" with digits must NOT emit outcome
    let false_positive_text = "Version 2 supports multiple platforms and Linux systems across 3 clusters.";
    let sha_fp = compute_sha256(false_positive_text);
    let (claims_fp, _) = build_substantive_research_package(
        "cand_fp",
        &sha_fp,
        "Version 2 supports multiple platforms and Linux systems across 3 clusters.",
        "Cluster Update",
        false_positive_text,
        None,
    );
    assert!(
        !claims_fp.iter().any(|c| c.statement.starts_with("Observed empirical outcome:")),
        "Ordinary words containing 'ms' substrings must NOT trigger false outcome claim"
    );
}

#[test]
fn test_stable_candidate_identity_across_content_updates() {
    let tmp_dir = std::env::temp_dir().join(format!("test_stable_id_{}", uuid::Uuid::new_v4()));
    let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp_dir).with_allow_http(true));
    let service = DailyNewsService::new(fetcher, storage.clone(), CandidateScorer::default())
        .with_source_capture(capturer);

    let raw_v1 = RawNewsItem::new(
        "item_42",
        "https://example.com/deep-dive-article",
        "Deep Dive into Memory Safety",
        "2026-11-04T05:00:00Z",
    );

    let mut raw_v2 = raw_v1.clone();
    raw_v2.body = Some("Substantial new technical content added by author in revision 2.".to_string());

    let cand_1 = service.normalize_item(&raw_v1).unwrap();
    let cand_2 = service.normalize_item(&raw_v2).unwrap();

    // 1. Candidate ID must remain IDENTICAL because canonical URL has not changed
    assert_eq!(
        cand_1.candidate_id, cand_2.candidate_id,
        "Candidate identity MUST remain stable across content revisions"
    );

    // 2. Content hash must DIFFER to accurately track the revision
    assert_ne!(
        cand_1.content_hash, cand_2.content_hash,
        "Content hash MUST change when body content is revised"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
