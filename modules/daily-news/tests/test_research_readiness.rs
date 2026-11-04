use daily_news::domain::{
    evaluate_readiness, ClaimEvidence, ContentOrigin, EvidenceRef, PrimarySourceEvidence,
    ReadinessStatus, ResearchReadiness, VisualEvidence,
};

#[test]
fn test_missing_primary_source_yields_needs_source() {
    let readiness = evaluate_readiness(None, &[], &[], ContentOrigin::HeadlineOnly);
    assert_eq!(readiness.status, ReadinessStatus::NeedsSource);
    assert!(readiness.reasons[0].contains("Primary article has not been captured"));
    assert_eq!(readiness.missing_fields, vec!["primary_source".to_string()]);
}

#[test]
fn test_captured_source_without_claims_yields_needs_research() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/article".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_type: Some("text/html".to_string()),
        title: Some("Article Title".to_string()),
        published_at: None,
        excerpt: Some("Captured excerpt of article".to_string()),
        snapshot_ref: Some(".state/evidence/abcd.txt".to_string()),
        snapshot_sha256: Some(
            "abcd1234ef567890abcd1234ef567890abcd1234ef567890abcd1234ef567890".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let readiness = evaluate_readiness(Some(&source), &[], &[], ContentOrigin::Article);
    assert_eq!(readiness.status, ReadinessStatus::NeedsResearch);
    assert_eq!(readiness.missing_fields, vec!["claims".to_string()]);
}

#[test]
fn test_claims_without_independent_check_yields_needs_verification() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement".to_string()),
        snapshot_ref: Some(".state/evidence/1234.txt".to_string()),
        snapshot_sha256: Some(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "4x latency improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            excerpt: "We measured 4x improvement".to_string(),
            locator: Some("Paragraph 3".to_string()),
        }],
        verification_status: "SOURCE_SUPPORTED".to_string(), // Source supported, but not independently verified!
        reviewer: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness(Some(&source), &claims, &visuals, ContentOrigin::Article);
    // T11: Article makes a claim, no independent check -> NEEDS_VERIFICATION not verified / video_ready
    assert_eq!(readiness.status, ReadinessStatus::NeedsVerification);
    assert!(readiness.reasons[0].contains("Independent verification has not been run"));
    assert_eq!(
        readiness.missing_fields,
        vec!["independent_fact_check".to_string()]
    );
}

#[test]
fn test_missing_visual_evidence_yields_needs_visual_evidence() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement".to_string()),
        snapshot_ref: Some(".state/evidence/1234.txt".to_string()),
        snapshot_sha256: Some(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "4x latency improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            excerpt: "We measured 4x improvement".to_string(),
            locator: Some("Paragraph 3".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("QA team".to_string()),
    }];

    // Missing visuals!
    let readiness = evaluate_readiness(Some(&source), &claims, &[], ContentOrigin::Article);
    // T13: Missing proof image/demo -> NEEDS_VISUAL_EVIDENCE
    assert_eq!(readiness.status, ReadinessStatus::NeedsVisualEvidence);
    assert_eq!(
        readiness.missing_fields,
        vec!["visual_evidence".to_string()]
    );
}

#[test]
fn test_all_criteria_met_yields_video_ready() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement".to_string()),
        snapshot_ref: Some(".state/evidence/1234.txt".to_string()),
        snapshot_sha256: Some(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "4x latency improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            excerpt: "We measured 4x improvement".to_string(),
            locator: Some("Paragraph 3".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("QA team".to_string()),
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness(Some(&source), &claims, &visuals, ContentOrigin::Article);
    assert_eq!(readiness.status, ReadinessStatus::VideoReady);
    assert!(readiness.missing_fields.is_empty());
}

#[test]
fn test_readiness_serde_roundtrip() {
    let original = ResearchReadiness {
        status: ReadinessStatus::NeedsVerification,
        reasons: vec!["Independent check pending".to_string()],
        missing_fields: vec!["independent_fact_check".to_string()],
        contract_version: "2.0".to_string(),
    };

    let serialized = serde_json::to_string(&original).expect("Must serialize");
    let deserialized: ResearchReadiness =
        serde_json::from_str(&serialized).expect("Must deserialize");
    assert_eq!(original, deserialized);
}

#[test]
fn test_unknown_enum_deserialization_fallback() {
    use daily_news::domain::PublishedAtKind;

    #[derive(serde::Deserialize)]
    struct Wrapper {
        origin: ContentOrigin,
        kind: PublishedAtKind,
    }

    let json_data = r#"{"origin": "future_unknown_origin", "kind": "unseen_publication_kind"}"#;
    let parsed: Wrapper =
        serde_json::from_str(json_data).expect("Must deserialize unknown enums gracefully");
    assert_eq!(parsed.origin, ContentOrigin::Unknown);
    assert_eq!(parsed.kind, PublishedAtKind::Unknown);
}
