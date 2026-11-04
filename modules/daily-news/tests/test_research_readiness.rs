use daily_news::domain::{
    compute_sha256, evaluate_readiness, evaluate_readiness_with_snapshot_text, ClaimEvidence,
    ContentOrigin, EvidenceRef, PrimarySourceEvidence, ReadinessStatus, ResearchReadiness,
    VerificationRecord, VisualEvidence,
};

#[test]
fn test_missing_primary_source_yields_needs_source() {
    let readiness = evaluate_readiness(None, None, &[], &[], ContentOrigin::HeadlineOnly);
    assert_eq!(readiness.status, ReadinessStatus::NeedsSource);
    assert!(readiness.reasons[0].contains("Primary article has not been captured"));
    assert_eq!(readiness.missing_fields, vec!["primary_source".to_string()]);
}

#[test]
fn test_captured_source_without_claims_yields_needs_research() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/article".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
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

    let readiness = evaluate_readiness(Some(&source), None, &[], &[], ContentOrigin::Article);
    assert_eq!(readiness.status, ReadinessStatus::NeedsResearch);
    assert_eq!(readiness.missing_fields, vec!["claims".to_string()]);
}

#[test]
fn test_claims_without_independent_check_yields_needs_verification() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
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
        verification_record: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness(Some(&source), None, &claims, &visuals, ContentOrigin::Article);
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
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
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
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    // Missing visuals!
    let readiness = evaluate_readiness(Some(&source), None, &claims, &[], ContentOrigin::Article);
    // T13: Missing proof image/demo -> NEEDS_VISUAL_EVIDENCE
    assert_eq!(readiness.status, ReadinessStatus::NeedsVisualEvidence);
    assert_eq!(
        readiness.missing_fields,
        vec!["visual_evidence".to_string()]
    );
}

#[test]
fn test_all_criteria_met_yields_video_ready() {
    let snapshot_text = "Benchmark Results: We measured 4x improvement across systems.";
    let sha256 = compute_sha256(snapshot_text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement across systems.".to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha256}.txt")),
        snapshot_sha256: Some(sha256.clone()),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "4x latency improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha256,
            excerpt: "We measured 4x improvement across systems.".to_string(),
            locator: Some("Paragraph 3".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(snapshot_text),
        &claims,
        &visuals,
        ContentOrigin::Article,
    );
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

#[test]
fn test_fake_unverified_claim_cannot_self_report_video_ready() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement in throughput.".to_string()),
        snapshot_ref: Some(".state/evidence/1234.txt".to_string()),
        snapshot_sha256: Some(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    // Self-reported independent verification without an accredited reviewer or independent source
    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "4x improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            excerpt: "We measured 4x improvement in throughput.".to_string(),
            locator: Some("p:2".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None, // No reviewer!
        verification_record: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness(Some(&source), None, &claims, &visuals, ContentOrigin::Article);
    assert_ne!(
        readiness.status,
        ReadinessStatus::VideoReady,
        "Raw input must NOT self-declare INDEPENDENTLY_VERIFIED without an accredited reviewer or secondary source"
    );
    assert_eq!(readiness.status, ReadinessStatus::NeedsVerification);
}

#[test]
fn test_snapshot_sha256_mismatch_fails_verification() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("Real article content".to_string()),
        snapshot_ref: Some(".state/evidence/abcd.txt".to_string()),
        snapshot_sha256: Some(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "Performance boost".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            excerpt: "Real article content".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    // Actual snapshot text has different SHA than declared
    let snapshot_text = "Some totally different content whose hash will not match all a's";
    let readiness = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(snapshot_text),
        &claims,
        &visuals,
        ContentOrigin::Article,
    );

    assert_eq!(readiness.status, ReadinessStatus::NeedsSource);
    assert!(readiness.reasons.iter().any(|r| r.contains("mismatch")));
}

#[test]
fn test_evidence_excerpt_missing_from_snapshot_fails() {
    use sha2::{Digest, Sha256};

    let actual_text = "Official release notes: We optimized memory allocation by 25%.";
    let actual_sha = Sha256::digest(actual_text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let source = PrimarySourceEvidence {
        url: "https://example.com/notes".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Release Notes".to_string()),
        published_at: None,
        excerpt: Some("We optimized memory allocation by 25%.".to_string()),
        snapshot_ref: Some(".state/evidence/notes.txt".to_string()),
        snapshot_sha256: Some(actual_sha.clone()),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    // Fabricated excerpt that doesn't appear in the text
    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "90% CPU drop".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: actual_sha,
            excerpt: "Fabricated claim: CPU usage dropped by 90% across all servers".to_string(),
            locator: Some("p:5".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(actual_text),
        &claims,
        &visuals,
        ContentOrigin::Article,
    );

    assert_eq!(readiness.status, ReadinessStatus::NeedsResearch);
    assert!(readiness.reasons.iter().any(|r| r.contains("not found in primary source")));
}

#[test]
fn test_visual_evidence_invalid_license_or_provenance_rejected() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement in throughput.".to_string()),
        snapshot_ref: Some(".state/evidence/1234.txt".to_string()),
        snapshot_sha256: Some(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "4x improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            excerpt: "We measured 4x improvement in throughput.".to_string(),
            locator: Some("p:2".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    // Restricted license
    let visuals_bad_license = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("all-rights-reserved".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let r1 = evaluate_readiness(Some(&source), None, &claims, &visuals_bad_license, ContentOrigin::Article);
    assert_eq!(r1.status, ReadinessStatus::NeedsVisualEvidence);

    // Dangerous javascript scheme in source_url
    let visuals_xss_url = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "javascript:alert(1)".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let r2 = evaluate_readiness(Some(&source), None, &claims, &visuals_xss_url, ContentOrigin::Article);
    assert_eq!(r2.status, ReadinessStatus::NeedsVisualEvidence);
}

#[test]
fn test_single_claim_independent_not_enough_if_other_claims_unverified() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement in throughput.".to_string()),
        snapshot_ref: Some(".state/evidence/1234.txt".to_string()),
        snapshot_sha256: Some(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        ),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![
        ClaimEvidence {
            claim_id: "c1".to_string(),
            statement: "4x improvement".to_string(),
            evidence_refs: vec![EvidenceRef {
                source_ref: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                    .to_string(),
                excerpt: "We measured 4x improvement in throughput.".to_string(),
                locator: Some("p:2".to_string()),
            }],
            verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
            reviewer: Some("Benchmarking Laboratory QA".to_string()),
            verification_record: None,
        },
        ClaimEvidence {
            claim_id: "c2".to_string(),
            statement: "Unverified wild claim".to_string(),
            evidence_refs: vec![],
            verification_status: "UNVERIFIED".to_string(),
            reviewer: None,
            verification_record: None,
        },
    ];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness(Some(&source), None, &claims, &visuals, ContentOrigin::Article);
    assert_ne!(readiness.status, ReadinessStatus::VideoReady);
    assert_eq!(readiness.status, ReadinessStatus::NeedsResearch);
}

#[test]
fn test_unverified_snapshot_text_fails_closed_cannot_yield_video_ready() {
    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
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
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    // When snapshot_text is None (e.g. raw JSON evaluate_readiness), CANNOT be VIDEO_READY!
    let readiness = evaluate_readiness(Some(&source), None, &claims, &visuals, ContentOrigin::Article);
    assert_ne!(readiness.status, ReadinessStatus::VideoReady);
    assert_eq!(readiness.status, ReadinessStatus::NeedsResearch);
    assert!(readiness.missing_fields.contains(&"verified_snapshot_text".to_string()));
}

#[test]
fn test_fake_reviewer_tokens_rejected_needs_verification() {
    let snapshot_text = "Benchmark Results: We measured 4x improvement across systems.";
    let sha256 = compute_sha256(snapshot_text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/benchmark".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Benchmark Results".to_string()),
        published_at: None,
        excerpt: Some("We measured 4x improvement across systems.".to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha256}.txt")),
        snapshot_sha256: Some(sha256.clone()),
        captured_at: Some("2026-11-03T12:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    // Fake / generic reviewer names that lack accredited institutions
    let fake_reviewers = vec![
        "QA team",
        "alice_qa",
        "Reviewer 1",
        "expert",
        "fake_reviewer",
        "test_qa",
    ];

    for fake_rev in fake_reviewers {
        let claims = vec![ClaimEvidence {
            claim_id: "c1".to_string(),
            statement: "4x latency improvement".to_string(),
            evidence_refs: vec![EvidenceRef {
                source_ref: sha256.clone(),
                excerpt: "We measured 4x improvement across systems.".to_string(),
                locator: Some("Paragraph 3".to_string()),
            }],
            verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
            reviewer: Some(fake_rev.to_string()),
            verification_record: None,
        }];

        let readiness = evaluate_readiness_with_snapshot_text(
            Some(&source),
            Some(snapshot_text),
            &claims,
            &visuals,
            ContentOrigin::Article,
        );
        assert_eq!(
            readiness.status,
            ReadinessStatus::NeedsVerification,
            "Fake reviewer '{fake_rev}' must be rejected with NEEDS_VERIFICATION"
        );
    }
}

#[test]
fn test_verification_record_structured_evidence() {
    let snapshot_text = "Verified cryptographic audit findings: All signature boundaries are hardened.";
    let sha256 = compute_sha256(snapshot_text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/audit".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Audit Report".to_string()),
        published_at: None,
        excerpt: Some("All signature boundaries are hardened.".to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha256}.txt")),
        snapshot_sha256: Some(sha256.clone()),
        captured_at: Some("2026-11-04T05:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let record = VerificationRecord {
        record_id: "vr_audit_001".to_string(),
        verified_at: "2026-11-04T05:00:00Z".to_string(),
        reviewer_id: "lead_auditor_42".to_string(),
        organization: "CertiK Security Laboratory".to_string(),
        method: "cryptographic_audit".to_string(),
        audit_trail_ref: Some("audit://certik/report_2026_001".to_string()),
        secondary_source_ref: None,
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c_audit_1".to_string(),
        statement: "All signature boundaries are hardened".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha256,
            excerpt: "All signature boundaries are hardened.".to_string(),
            locator: Some("section:findings".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(record),
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_audit_diagram".to_string(),
        kind: "diagram".to_string(),
        source_url: "https://example.com/diagram.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("licensed".to_string()),
        provenance_status: Some("verified".to_string()),
    }];

    let readiness = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(snapshot_text),
        &claims,
        &visuals,
        ContentOrigin::Article,
    );
    assert_eq!(readiness.status, ReadinessStatus::VideoReady);
}

#[test]
fn test_invalid_visual_assets_ssrf_and_placeholders_rejected() {
    let snapshot_text = "Official release: New benchmark results published.";
    let sha256 = compute_sha256(snapshot_text);

    let source = PrimarySourceEvidence {
        url: "https://example.com/notes".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Release".to_string()),
        published_at: None,
        excerpt: Some("New benchmark results published.".to_string()),
        snapshot_ref: Some(format!(".state/evidence/{sha256}.txt")),
        snapshot_sha256: Some(sha256.clone()),
        captured_at: Some("2026-11-04T05:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    };

    let claims = vec![ClaimEvidence {
        claim_id: "c1".to_string(),
        statement: "Benchmark results".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: sha256,
            excerpt: "New benchmark results published.".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    // 1. SSRF target (AWS metadata IP)
    let visuals_ssrf = vec![VisualEvidence {
        asset_ref: "vis_ssrf".to_string(),
        kind: "chart".to_string(),
        source_url: "http://169.254.169.254/latest/meta-data/img.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];
    let r1 = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(snapshot_text),
        &claims,
        &visuals_ssrf,
        ContentOrigin::Article,
    );
    assert_eq!(r1.status, ReadinessStatus::NeedsVisualEvidence);

    // 2. Placeholder asset_ref
    let visuals_placeholder = vec![VisualEvidence {
        asset_ref: "placeholder".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];
    let r2 = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(snapshot_text),
        &claims,
        &visuals_placeholder,
        ContentOrigin::Article,
    );
    assert_eq!(r2.status, ReadinessStatus::NeedsVisualEvidence);

    // 3. Directory traversal in asset_ref
    let visuals_traversal = vec![VisualEvidence {
        asset_ref: "../../etc/shadow.png".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
    }];
    let r3 = evaluate_readiness_with_snapshot_text(
        Some(&source),
        Some(snapshot_text),
        &claims,
        &visuals_traversal,
        ContentOrigin::Article,
    );
    assert_eq!(r3.status, ReadinessStatus::NeedsVisualEvidence);
}
