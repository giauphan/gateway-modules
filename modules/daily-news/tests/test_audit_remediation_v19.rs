use daily_news::domain::{
    compute_sha256, ClaimEvidence, ContentOrigin, EvidenceRef, NormalizedCandidate,
    PrimarySourceEvidence, ReadinessStatus, VisualEvidence,
};
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::FileStorageAdapter;
use daily_news::{RawNewsItem, StoragePort};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

#[test]
fn test_ten_candidates_enrich_and_restart_persistence() {
    let tmp_dir = std::env::temp_dir().join(format!("test_ten_cands_{}", uuid::Uuid::new_v4()));
    let (cand_1_id, cand_2_id) = ("cand_test_01".to_string(), "cand_test_02".to_string());

    // 1. Initial run: setup server for candidate 1 & 2 capture
    let listener = TcpListener::bind("127.0.0.1:0").expect("Bind ephemeral server");
    let port = listener.local_addr().unwrap().port();

    let server_handle = thread::spawn(move || {
        for _ in 0..2 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "<html><head><title>Distributed Systems Release</title></head><body><p>Identified high memory bottleneck under load. We implemented a lock-free ring buffer algorithm. Benchmarks demonstrate a 4x reduction in cache latency.</p></body></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    });

    {
        let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
        let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp_dir).with_allow_http(true));
        let fetcher = Arc::new(LiveFeedFetcher::new(None));
        let scorer = CandidateScorer::default();
        let service = DailyNewsService::new(fetcher, storage.clone(), scorer).with_source_capture(capturer);

        // Populate 10 candidates
        let mut candidates = Vec::new();
        for i in 1..=10 {
            let mut c = NormalizedCandidate::default();
            c.candidate_id = format!("cand_test_{i:02}");
            c.title = format!("Candidate Article {i}");
            c.canonical_url = format!("http://127.0.0.1:{port}/article_{i}");
            c.source_url = c.canonical_url.clone();
            c.url = c.canonical_url.clone();
            c.published_at = "2026-11-04T05:00:00Z".to_string();
            c.published_at_utc = "2026-11-04T05:00:00Z".to_string();
            c.content_origin = ContentOrigin::HeadlineOnly;
            candidates.push(c);
        }
        service.consume(&[]).unwrap();
        // Upsert 10 candidates
        storage.cache_candidates(&candidates);
        assert_eq!(storage.get_cached_candidates().len(), 10);

        // Enrich Candidate 1 -> capture source (transitions to NEEDS_VERIFICATION)
        let enriched_1 = service
            .enrich_candidate(&cand_1_id, None, None, None)
            .expect("Enrich cand 1");
        assert_eq!(
            enriched_1.candidate.research_readiness.as_ref().unwrap().status,
            ReadinessStatus::NeedsVerification
        );
        let primary_sha = enriched_1.candidate.primary_source.as_ref().unwrap().snapshot_sha256.clone().unwrap();

        // Enrich Candidate 2 -> with accredited reviewer and visual evidence -> VIDEO_READY
        let audit_content = "Official Cryptographic Audit Trail Report #2026-01 from CertiK (https://certik.com/reports/benchmark ref #: 2026-01): CertiK independently verified findings confirmed for 4x reduction in cache latency across all test runs.";
        let audit_sha = compute_sha256(audit_content);
        let verified_claims = vec![ClaimEvidence {
            claim_id: "claim_verified_02".to_string(),
            statement: "4x reduction in cache latency".to_string(),
            evidence_refs: vec![EvidenceRef {
                source_ref: primary_sha,
                excerpt: "Benchmarks demonstrate a 4x reduction in cache latency.".to_string(),
                locator: Some("section:outcome".to_string()),
            }],
            verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
            reviewer: None,
            verification_record: Some(daily_news::domain::VerificationRecord {
                record_id: "vr_test_cand_02".to_string(),
                reviewer_id: "certik_lead_auditor".to_string(),
                organization: "CertiK".to_string(),
                method: "reproducible_benchmark".to_string(),
                verified_at: "2026-11-04T05:00:00Z".to_string(),
                audit_trail_ref: Some(audit_sha.clone()),
                secondary_source_ref: Some("https://certik.com/reports/benchmark".to_string()),
            }),
        }];

        let chart_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
        let chart_sha = daily_news::url::compute_sha256_bytes(chart_bytes);
        let evidence_dir = tmp_dir.join(".state").join("evidence");
        let assets_dir = tmp_dir.join(".state").join("assets");
        let _ = std::fs::create_dir_all(&evidence_dir);
        let _ = std::fs::create_dir_all(&assets_dir);
        let _ = std::fs::write(evidence_dir.join(format!("{chart_sha}.txt")), chart_bytes);
        let _ = std::fs::write(assets_dir.join(format!("{chart_sha}.bin")), chart_bytes);
        let _ = std::fs::write(evidence_dir.join(format!("{audit_sha}.txt")), audit_content.as_bytes());

        let license_text = "Copyright 2026 Example Corp. Licensed under CC-BY for commercial use and distribution of vis_benchmark_chart.";
        let lic_sha = compute_sha256("https://example.com/license");
        let _ = std::fs::write(evidence_dir.join(format!("{lic_sha}.txt")), license_text.as_bytes());

        let visuals = vec![VisualEvidence {
            asset_ref: "vis_benchmark_chart".to_string(),
            kind: "chart".to_string(),
            source_url: "https://example.com/benchmark_chart.png".to_string(),
            source_snapshot_ref: None,
            license_status: Some("cc-by".to_string()),
            provenance_status: Some("verified".to_string()),
            checksum_sha256: Some(chart_sha),
            license_evidence_ref: Some("https://example.com/license".to_string()),
            verified_rights: Some(true),
        }];

        let enriched_2 = service
            .enrich_candidate(&cand_2_id, None, Some(verified_claims), Some(visuals))
            .expect("Enrich cand 2 to video ready");
        assert_eq!(
            enriched_2.candidate.research_readiness.as_ref().unwrap().status,
            ReadinessStatus::VideoReady
        );
        assert!(enriched_2.production_scoring.is_video_production_ready);

        // Verify that candidates 3 through 10 remain intact in cache
        let in_memory = storage.get_cached_candidates();
        assert_eq!(in_memory.len(), 10, "All 10 candidates must remain in cache");
    }

    // 2. SIMULATE RESTART: Instantiate a new FileStorageAdapter pointing to same tmp_dir
    {
        let restarted_storage = FileStorageAdapter::new(&tmp_dir);
        let recovered = restarted_storage.get_cached_candidates();
        assert_eq!(
            recovered.len(),
            10,
            "CRITICAL ACCEPTANCE: All 10 candidates must survive restart"
        );

        // Verify Candidate 1 state survived
        let c1 = recovered.iter().find(|c| c.candidate_id == cand_1_id).expect("Find cand 1");
        assert_eq!(
            c1.research_readiness.as_ref().unwrap().status,
            ReadinessStatus::NeedsVerification,
            "Candidate 1 must preserve NEEDS_VERIFICATION"
        );

        // Verify Candidate 2 state survived as VIDEO_READY
        let c2 = recovered.iter().find(|c| c.candidate_id == cand_2_id).expect("Find cand 2");
        assert_eq!(
            c2.research_readiness.as_ref().unwrap().status,
            ReadinessStatus::VideoReady,
            "Candidate 2 must preserve VIDEO_READY"
        );
        assert_eq!(c2.claims.len(), 1);
        assert_eq!(c2.claims[0].verification_status, "INDEPENDENTLY_VERIFIED");
        assert_eq!(c2.visual_evidence.len(), 1);

        // Verify Candidate 3 survived in original state
        let c3 = recovered.iter().find(|c| c.candidate_id == "cand_test_03").expect("Find cand 3");
        assert_eq!(c3.title, "Candidate Article 3");
    }

    let _ = server_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_curated_tier_immediate_enrichment() {
    let tmp_dir = std::env::temp_dir().join(format!("test_curated_enrich_{}", uuid::Uuid::new_v4()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("Bind ephemeral server");
    let port = listener.local_addr().unwrap().port();

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = "<html><head><title>Curated Article</title></head><body><p>Curated technical release: We optimized consensus round latency across distributed validator nodes. The new protocol pipeline processes voting packets asynchronously, avoiding round synchronization stalls during peak transaction volume.</p></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });

    let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp_dir).with_allow_http(true));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer).with_source_capture(capturer);

    let mut curated_item = RawNewsItem::new(
        "curated_001",
        format!("http://127.0.0.1:{port}/curated"),
        "Curated Consensus Round Latency Optimization",
        daily_news::domain::current_utc_timestamp(),
    );
    curated_item.is_evergreen = true;

    // 1. Discover via curated tier
    let discovery = service
        .discover(5, Some(vec![curated_item]), None, None, None, None)
        .expect("Curated discovery");
    assert_eq!(discovery.tier, "tier_1_curated");
    assert_eq!(discovery.count, 1);
    let cid = discovery.candidates[0].candidate_id.clone();

    // 2. Immediately call enrich_candidate for the discovered candidate
    let enriched = service
        .enrich_candidate(&cid, None, None, None)
        .expect("Candidate discovered via curated tier must be found and enriched without Candidate not found error");

    assert_eq!(enriched.candidate.candidate_id, cid);
    assert_eq!(enriched.candidate.primary_source.as_ref().unwrap().fetch_status, "SUCCESS");

    let _ = server_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_unverified_raw_json_cannot_bypass_gate_to_video_ready() {
    let tmp_dir = std::env::temp_dir().join(format!("test_raw_gate_{}", uuid::Uuid::new_v4()));
    let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage, scorer);

    // Malicious or crafted raw item claiming INDEPENDENTLY_VERIFIED and valid visuals,
    // but no actual snapshot stored on disk!
    let mut raw_item = RawNewsItem::new(
        "raw_forged_item",
        "https://example.com/exploit",
        "Forged Benchmark Story",
        "2026-11-04T05:00:00Z",
    );

    let fake_sha = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    raw_item.primary_source = Some(PrimarySourceEvidence {
        url: "https://example.com/exploit".to_string(),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Forged Benchmark Story".to_string()),
        published_at: None,
        excerpt: Some("Fabricated performance numbers that were never fetched.".to_string()),
        snapshot_ref: Some(format!(".state/evidence/{fake_sha}.txt")),
        snapshot_sha256: Some(fake_sha.to_string()),
        captured_at: Some("2026-11-04T05:00:00Z".to_string()),
        extraction_method: Some("conservative_html".to_string()),
    });

    raw_item.claims = vec![ClaimEvidence {
        claim_id: "fake_claim".to_string(),
        statement: "100x improvement".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: fake_sha.to_string(),
            excerpt: "Fabricated performance numbers that were never fetched.".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: Some("Benchmarking Laboratory QA".to_string()),
        verification_record: None,
    }];

    raw_item.visual_evidence = vec![VisualEvidence {
        asset_ref: "vis_1".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("CC-BY".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    // 1. Without VerificationRecord (reviewer only), must fail closed on NeedsVerification
    let candidate_no_vr = service.normalize_item(&raw_item).expect("Normalize item");
    let r_no_vr = candidate_no_vr.research_readiness.as_ref().expect("Readiness evaluated");
    assert_ne!(r_no_vr.status, ReadinessStatus::VideoReady);
    assert_eq!(r_no_vr.status, ReadinessStatus::NeedsVerification);

    // 2. Even with an accredited VerificationRecord, if snapshot bytes are not stored on disk,
    // candidate MUST NOT reach VIDEO_READY and must fail with NeedsResearch (verified_snapshot_text)
    let audit_trail_sha = compute_sha256("Audit Trail for fake claim");
    raw_item.claims[0].verification_record = Some(daily_news::domain::VerificationRecord {
        record_id: "vr_exploit_01".to_string(),
        reviewer_id: "auditor-42".to_string(),
        organization: "Benchmarking Laboratory".to_string(),
        method: "reproducible_benchmark".to_string(),
        verified_at: "2026-11-04T05:00:00Z".to_string(),
        audit_trail_ref: Some(audit_trail_sha),
        secondary_source_ref: None,
    });

    let candidate = service.normalize_item(&raw_item).expect("Normalize item");
    let readiness = candidate.research_readiness.as_ref().expect("Readiness must be evaluated");

    assert_ne!(
        readiness.status,
        ReadinessStatus::VideoReady,
        "Forged raw item must NEVER reach VIDEO_READY without verified snapshot bytes"
    );
    assert!(readiness.status == ReadinessStatus::NeedsVerification || readiness.status == ReadinessStatus::NeedsResearch);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_substantive_research_packaging_structure() {
    let text = "Identifying challenges: High lock contention was identified as a major bottleneck in processing transactions. To address this, we implemented a lock-free partition architecture. Benchmarks demonstrate a 3.5x reduction in latency under peak concurrency.";
    let sha = compute_sha256(text);

    let (claims, storyboard) = daily_news::enrichment::build_substantive_research_package(
        "cand_substantive_test",
        &sha,
        text,
        "Lock-Free Partitioning Engine",
        text,
        Some(".state/evidence/lockfree.txt"),
    );

    // Verify claims are structured with problem, solution, and outcome
    assert!(claims.len() >= 2, "Must build at least problem and solution claims");
    assert!(claims.iter().any(|c| c.claim_id.contains("problem")));
    assert!(claims.iter().any(|c| c.claim_id.contains("solution")));
    assert!(claims.iter().any(|c| c.claim_id.contains("outcome")));

    for claim in &claims {
        assert_eq!(claim.verification_status, "SOURCE_SUPPORTED");
        assert!(!claim.evidence_refs.is_empty());
        assert_eq!(claim.evidence_refs[0].source_ref, sha);
        // Excerpt must be present in the text!
        assert!(text.contains(&claim.evidence_refs[0].excerpt));
    }

    // Verify storyboard
    assert!(storyboard.hook.is_some());
    assert!(storyboard.viewer_question.is_some());
    assert!(storyboard.symptom.is_some());
    assert!(storyboard.remedy.is_some());
    assert_eq!(storyboard.snapshot_sha256.as_deref(), Some(sha.as_str()));
}

#[test]
fn test_rediscovery_preserves_enriched_candidate_state() {
    let tmp_dir = std::env::temp_dir().join(format!("test_rediscovery_{}", uuid::Uuid::new_v4()));
    let storage = FileStorageAdapter::new(&tmp_dir);

    let mut candidate = NormalizedCandidate {
        candidate_id: "cand_rediscovery_test".to_string(),
        title: "Enriched Candidate".to_string(),
        ..Default::default()
    };
    candidate.research.primary_source = Some(PrimarySourceEvidence {
        url: "https://example.com/item".to_string(),
        snapshot_ref: Some(".state/evidence/item.html".to_string()),
        snapshot_sha256: Some("a".repeat(64)),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("PASSED".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Item Title".to_string()),
        published_at: Some("2026-11-04T00:00:00Z".to_string()),
        excerpt: Some("Excerpt".to_string()),
        captured_at: Some("2026-11-04T00:00:00Z".to_string()),
        extraction_method: Some("readability".to_string()),
    });
    candidate.research.claims = vec![ClaimEvidence {
        claim_id: "claim_01".to_string(),
        statement: "Verified claim".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "a".repeat(64),
            excerpt: "Verified claim excerpt".to_string(),
            locator: Some("sec:1".to_string()),
        }],
        verification_status: "SOURCE_SUPPORTED".to_string(),
        reviewer: None,
        verification_record: None,
    }];
    candidate.research.research_readiness = Some(daily_news::domain::ResearchReadiness {
        status: ReadinessStatus::NeedsVerification,
        reasons: vec!["Awaiting independent check".to_string()],
        missing_fields: vec!["independent_verification".to_string()],
        contract_version: "2.0".to_string(),
    });

    // 1. Initial upsert of enriched candidate
    storage.upsert_candidates(&[candidate.clone()]);
    let initial = storage.get_cached_candidates();
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].research.claims.len(), 1);

    // 2. Incoming raw discovery of same candidate_id with empty research and default readiness
    let raw_rediscovered = NormalizedCandidate {
        candidate_id: "cand_rediscovery_test".to_string(),
        title: "Enriched Candidate (Updated Feed Title)".to_string(),
        research: daily_news::domain::ResearchEvidenceFields {
            research_readiness: Some(daily_news::domain::ResearchReadiness {
                status: ReadinessStatus::NeedsSource,
                reasons: vec!["Raw item".to_string()],
                missing_fields: vec!["primary_source".to_string()],
                contract_version: "2.0".to_string(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    storage.upsert_candidates(&[raw_rediscovered]);
    let after_rediscovery = storage.get_cached_candidates();
    assert_eq!(after_rediscovery.len(), 1);
    let preserved = &after_rediscovery[0];

    // Enrichment must NOT be wiped by raw rediscovery!
    assert!(preserved.research.primary_source.is_some(), "Primary source must be preserved");
    assert_eq!(preserved.research.claims.len(), 1, "Claims must be preserved");
    assert_eq!(
        preserved.research.research_readiness.as_ref().unwrap().status,
        ReadinessStatus::NeedsVerification,
        "Readiness status must be preserved"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_secondary_source_ref_must_match_strictly() {
    let snapshot_text = "Standard primary article body with verified benchmarks.";
    let primary_sha = compute_sha256(snapshot_text);
    let secondary_text = "Secondary laboratory reproduction report: Standard primary article body with verified benchmarks.";
    let secondary_sha_actual = compute_sha256(secondary_text);
    let secondary_sha_mismatched = "3".repeat(64);
    let audit_trail_sha = compute_sha256("Audit Trail: Laboratory cryptographic benchmark report #2026-01");

    let primary_source = PrimarySourceEvidence {
        url: "https://example.com/paper".to_string(),
        snapshot_ref: Some(".state/evidence/paper.html".to_string()),
        snapshot_sha256: Some(primary_sha.clone()),
        fetch_status: "SUCCESS".to_string(),
        content_quality: Some("PASSED".to_string()),
        content_type: Some("text/html".to_string()),
        title: Some("Paper Title".to_string()),
        published_at: Some("2026-11-04T00:00:00Z".to_string()),
        excerpt: Some("Excerpt".to_string()),
        captured_at: Some("2026-11-04T00:00:00Z".to_string()),
        extraction_method: Some("readability".to_string()),
    };

    // Claim cites secondary_sha_actual in evidence_refs, but verification record points to secondary_sha_mismatched
    let mismatched_claim = ClaimEvidence {
        claim_id: "claim_sec_test".to_string(),
        statement: "Independently verified claim".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: secondary_sha_actual.clone(),
            excerpt: "Standard primary article body with verified benchmarks.".to_string(),
            locator: Some("sec:lab".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(daily_news::domain::VerificationRecord {
            record_id: "vr_test_01".to_string(),
            reviewer_id: "reviewer-benchmarking-lab".to_string(),
            organization: "Benchmarking Laboratory".to_string(),
            method: "reproducible_benchmark".to_string(),
            verified_at: "2026-11-04T00:00:00Z".to_string(),
            audit_trail_ref: Some(audit_trail_sha),
            secondary_source_ref: Some(secondary_sha_mismatched), // Mismatch!
        }),
    };

    let visuals = vec![VisualEvidence {
        asset_ref: "chart_01".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let sec_text_owned = secondary_text.to_string();
    let sec_sha_owned = secondary_sha_actual.clone();
    let resolver = move |hash: &str| {
        if hash == sec_sha_owned {
            Some(sec_text_owned.clone())
        } else {
            None
        }
    };

    // 1. Mismatched secondary source ref fails
    let readiness = daily_news::research_readiness::evaluate_readiness_with_snapshot_store(
        Some(&primary_source),
        Some(snapshot_text),
        std::slice::from_ref(&mismatched_claim),
        &visuals,
        ContentOrigin::Article,
        Some(&resolver),
    );
    assert_eq!(
        readiness.status,
        ReadinessStatus::NeedsResearch,
        "Mismatched secondary source ref must fail claim grounding"
    );
    assert!(readiness.missing_fields.contains(&"valid_evidence_refs".to_string()));

    // 2. Matching secondary source ref but NO artifact resolver fails closed
    let mut matched_claim = mismatched_claim;
    matched_claim.verification_record.as_mut().unwrap().secondary_source_ref = Some(secondary_sha_actual);
    let no_resolver_readiness = daily_news::research_readiness::evaluate_readiness_with_snapshot_store(
        Some(&primary_source),
        Some(snapshot_text),
        &[matched_claim.clone()],
        &visuals,
        ContentOrigin::Article,
        None,
    );
    assert_eq!(
        no_resolver_readiness.status,
        ReadinessStatus::NeedsResearch,
        "Missing secondary snapshot artifact must fail claim grounding"
    );

    // 3. Positive control: secondary artifact bytes match, but without audit/asset resolvers, holds at NeedsVerification
    let matched_readiness = daily_news::research_readiness::evaluate_readiness_with_snapshot_store(
        Some(&primary_source),
        Some(snapshot_text),
        &[matched_claim],
        &visuals,
        ContentOrigin::Article,
        Some(&resolver),
    );
    assert_ne!(
        matched_readiness.status,
        ReadinessStatus::VideoReady,
        "Cannot reach VIDEO_READY without audit and asset resolvers"
    );
    assert_eq!(matched_readiness.status, ReadinessStatus::NeedsVerification);
}

#[test]
fn test_substantive_research_html_stripping_and_decimal_tokens() {
    let html = "<html><head><title>System Benchmark</title><style>.bad{color:red;}</style></head><body><p>Detected severe throughput bottleneck under burst load.</p><script>console.log('risk');</script><p>We introduced an asynchronous batch processor. Achieved a 3.5x speedup and 40% reduction in CPU overhead.</p></body></html>";
    let sha = compute_sha256(html);

    let (claims, storyboard) = daily_news::enrichment::build_substantive_research_package(
        "cand_html_test",
        &sha,
        "Detected severe throughput bottleneck under burst load.",
        "System Benchmark",
        html,
        Some(".state/evidence/benchmark.html"),
    );

    for claim in &claims {
        assert!(!claim.statement.contains('<'), "Statement must not contain HTML tags: {}", claim.statement);
        assert!(!claim.statement.contains('>'), "Statement must not contain HTML tags: {}", claim.statement);
        assert!(!claim.evidence_refs[0].excerpt.contains('<'), "Excerpt must not contain HTML tags");
        assert!(!claim.statement.contains("console.log"), "Script content must not be included");
    }

    assert!(!storyboard.symptom.as_ref().unwrap().contains('<'));
    assert!(!storyboard.remedy.as_ref().unwrap().contains('<'));
}
