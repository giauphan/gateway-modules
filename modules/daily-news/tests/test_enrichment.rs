use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, NormalizedCandidate, RawProvenance,
    ReadinessStatus, VisualEvidence,
};
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::FileStorageAdapter;
use daily_news::StoragePort;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

#[test]
fn test_enrich_candidate_workflow_state_transitions() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral server");
    let port = listener.local_addr().unwrap().port();

    let server_handle = thread::spawn(move || {
        // Serve 2 requests: 1st for initial capture, 2nd for re-enrichment
        for _ in 0..2 {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "<html><head><title>Deep Learning Compiler</title></head><body><p>We demonstrated a 4x reduction in cache latency using deterministic memory management.</p></body></html>";
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

    let tmp_dir = std::env::temp_dir().join(format!("test_enrich_wf_{port}"));
    let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp_dir).with_allow_http(true));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();

    let service = DailyNewsService::new(fetcher, storage.clone(), scorer)
        .with_source_capture(capturer);

    let initial_candidate = NormalizedCandidate {
        candidate_id: "cand_wf_test_1".to_string(),
        canonical_url: format!("http://127.0.0.1:{port}/article"),
        original_url: format!("http://127.0.0.1:{port}/article"),
        url: format!("http://127.0.0.1:{port}/article"),
        source_url: format!("http://127.0.0.1:{port}/article"),
        source_type: "news".to_string(),
        title: "Deep Learning Compiler Latency Reduction".to_string(),
        publisher: "techcrunch.com".to_string(),
        normalized_publisher: "techcrunch".to_string(),
        published_at_utc: "2026-11-04T05:00:00Z".to_string(),
        published_at: "2026-11-04T05:00:00Z".to_string(),
        content_hash: "chash123".to_string(),
        title_hash: "thash123".to_string(),
        raw_provenance: RawProvenance::default(),
        adapter_version: "1.8.0".to_string(),
        topic_family: "ai".to_string(),
        author: None,
        description: None,
        body: None,
        summary: None,
        tags: vec!["ai".to_string()],
        language: "en".to_string(),
        image_url: None,
        is_evergreen: false,
        content_kind: "news".to_string(),
        updated_at: None,
        discussion_url: None,
        evidence: Default::default(),
        independent_fact_check: None,
        scoring: None,
        content_origin: ContentOrigin::HeadlineOnly,
        published_at_kind: "article_publication".to_string(),
        research: Default::default(),
    };

    // Cache initial candidate in NEEDS_SOURCE state
    storage.cache_candidates(&[initial_candidate]);

    // 1. Initial enrichment -> captures source and grounds initial claim
    let enriched_res = service
        .enrich_candidate("cand_wf_test_1", None, None, None)
        .expect("Enrichment should succeed");

    assert!(enriched_res.candidate.primary_source.is_some());
    let source = enriched_res.candidate.primary_source.as_ref().unwrap();
    assert_eq!(source.fetch_status, "SUCCESS");
    assert_eq!(source.content_quality.as_deref(), Some("ARTICLE_CONTENT_VALID"));
    assert!(source.snapshot_sha256.is_some());
    let primary_sha = source.snapshot_sha256.as_ref().unwrap().clone();

    // With initial source-supported claim, status is NEEDS_VERIFICATION (not video ready yet)
    let readiness = enriched_res.candidate.research_readiness.as_ref().unwrap();
    assert_eq!(readiness.status, ReadinessStatus::NeedsVerification);

    // 2. Second stage enrichment: provide verified claims with accredited reviewer + CC-BY visual asset
    let verified_claims = vec![ClaimEvidence {
        claim_id: "claim_verified_1".to_string(),
        statement: "4x reduction in cache latency".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha,
            excerpt: "4x reduction in cache latency using deterministic memory management.".to_string(),
            locator: Some("paragraph:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(daily_news::domain::VerificationRecord {
            record_id: "rec_enrich_01".to_string(),
            verified_at: "2026-11-04T05:00:00Z".to_string(),
            reviewer_id: "benchmarking_auditor".to_string(),
            organization: "Benchmarking Laboratory".to_string(),
            method: "reproducible_benchmark".to_string(),
            audit_trail_ref: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()),
            secondary_source_ref: None,
        }),
    }];

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_compiler_benchmark".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/compiler_chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let video_ready_res = service
        .enrich_candidate(
            "cand_wf_test_1",
            None,
            Some(verified_claims),
            Some(visuals),
        )
        .expect("Video ready enrichment should succeed");

    let final_readiness = video_ready_res.candidate.research_readiness.as_ref().unwrap();
    assert_eq!(final_readiness.status, ReadinessStatus::VideoReady);
    assert!(video_ready_res.production_scoring.is_video_production_ready);
    assert_eq!(video_ready_res.production_scoring.verified_claim_count, 1);
    assert_eq!(video_ready_res.production_scoring.independent_confirmation_count, 1);
    assert_eq!(video_ready_res.production_scoring.visual_asset_readiness, 1.0);

    // Verify candidate is persistently updated in storage
    let cached_after = storage.get_cached_candidates();
    let retrieved = cached_after
        .iter()
        .find(|c| c.candidate_id == "cand_wf_test_1")
        .expect("Must find candidate in cache");
    assert_eq!(
        retrieved.research_readiness.as_ref().unwrap().status,
        ReadinessStatus::VideoReady
    );

    let _ = server_handle.join();
    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn test_enrich_nonexistent_candidate_fails() {
    let tmp_dir = std::env::temp_dir().join("test_enrich_nonexistent");
    let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp_dir));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();

    let service = DailyNewsService::new(fetcher, storage, scorer)
        .with_source_capture(capturer);

    let res = service.enrich_candidate("cand_does_not_exist", None, None, None);
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("Candidate not found"));

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
