//! Regression test suite for v1.19.3 remediation:
//! 1. P0: Audit provenance authenticity (authentic report markers, accredited issuer domain, ResolvedDocument vs IssuerAuthenticated).
//! 2. P0: License restrictive clause rejection ("License required before commercial use...", "Purchase a license...", etc.).
//! 3. P1: OS-level kernel advisory file lock (concurrency, non-deletion of active locks, drop release).
//! 4. P1: Transactional handoff commit & state reconciliation across restarts.
//! 5. P1: Cached readiness revalidation in-place persistence across restart (retrieve -> downgrade -> persist -> restart -> retrieve).
//! 6. P1: MCP tools schema exposure for complete handoff lifecycle (verify, release, commit).

use daily_news::date::current_utc_timestamp;
use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, NormalizedCandidate, PrimarySourceEvidence,
    ReadinessStatus, ResearchEvidenceFields, VerificationRecord, VisualEvidence,
};
use daily_news::ports::StoragePort;
use daily_news::research_readiness::{
    check_audit_provenance_level_with_resolver, is_audit_source_verified_with_resolver,
    is_visual_rights_verified, AuditProvenanceStatus,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::{FileStorageAdapter, ProcessFileLock};
use daily_news::url::compute_sha256;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dn_v1193_{tag}_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

struct MockFeedFetcher {
    items: Vec<daily_news::domain::RawNewsItem>,
}

impl daily_news::ports::FeedFetcherPort for MockFeedFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<daily_news::domain::RawNewsItem>, String> {
        Ok(self.items.clone())
    }
    fn fetch_sources(
        &self,
        _sources: &[String],
        _limit: usize,
    ) -> Result<Vec<daily_news::domain::RawNewsItem>, String> {
        Ok(self.items.clone())
    }
}

// ---------------------------------------------------------------------------
// 1. P0: License Restriction Clause Rejection
// ---------------------------------------------------------------------------
#[test]
fn test_license_restriction_clauses_fail_closed() {
    let visual_asset = VisualEvidence {
        asset_ref: "chart.png".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/assets/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(compute_sha256("chart_bytes")),
        license_evidence_ref: Some("https://example.com/legal/license".to_string()),
        verified_rights: Some(true),
    };

    // A. The exact reported finding: "License required before commercial use of chart.png. Copyright 2026 Example Corp."
    // MUST FAIL CLOSED!
    let restrictive_doc_1 = "License required before commercial use of chart.png. Copyright 2026 Example Corp.";
    let res_1 = move |_r: &str| -> Option<String> { Some(restrictive_doc_1.to_string()) };
    assert!(
        !is_visual_rights_verified(&visual_asset, Some(&res_1)),
        "Must reject document stating 'license required before commercial use'"
    );

    // B. "All rights reserved. Unauthorized reproduction prohibited. Example Inc."
    // MUST FAIL CLOSED!
    let restrictive_doc_2 = "All rights reserved. Unauthorized commercial use prohibited for chart.png. Example Corp.";
    let res_2 = move |_r: &str| -> Option<String> { Some(restrictive_doc_2.to_string()) };
    assert!(
        !is_visual_rights_verified(&visual_asset, Some(&res_2)),
        "Must reject document with 'all rights reserved' or 'unauthorized prohibited'"
    );

    // C. "Purchase a license before using chart.png. Contact license@example.com."
    // MUST FAIL CLOSED!
    let restrictive_doc_3 = "Purchase a license before using chart.png. Contact us for licensing. Example Corp.";
    let res_3 = move |_r: &str| -> Option<String> { Some(restrictive_doc_3.to_string()) };
    assert!(
        !is_visual_rights_verified(&visual_asset, Some(&res_3)),
        "Must reject document requiring license purchase"
    );

    // D. Authentic Affirmative Grant:
    // "chart.png is released under Creative Commons Attribution (CC-BY). Free for commercial use by Example Corp."
    // SUCCEEDS!
    let grant_doc = "chart.png is released under Creative Commons Attribution (CC-BY). Free for commercial use by Example Corp.";
    let res_grant = move |_r: &str| -> Option<String> { Some(grant_doc.to_string()) };
    assert!(
        is_visual_rights_verified(&visual_asset, Some(&res_grant)),
        "Must accept authentic affirmative grant without restrictive clauses"
    );

    // E. Standard MIT License text with 'NONINFRINGEMENT' and 'permission is hereby granted':
    // SUCCEEDS!
    let mit_doc = "MIT License for chart.png. Copyright (c) 2026 Example Corp. Permission is hereby granted, free of charge, to any person obtaining a copy. THE SOFTWARE IS PROVIDED AS IS, WITHOUT WARRANTY OF ANY KIND, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.";
    let res_mit = move |_r: &str| -> Option<String> { Some(mit_doc.to_string()) };
    assert!(
        is_visual_rights_verified(&visual_asset, Some(&res_mit)),
        "Must accept standard MIT license text containing 'NONINFRINGEMENT' without false rejection"
    );
}

// ---------------------------------------------------------------------------
// 2. P0: Audit Provenance: ResolvedDocument vs IssuerAuthenticated
// ---------------------------------------------------------------------------
#[test]
fn test_audit_provenance_distinguishes_resolved_doc_from_authenticated_issuer() {
    let rec = VerificationRecord {
        record_id: "certik_2026_101".to_string(),
        verified_at: "2026-11-04T00:00:00Z".to_string(),
        reviewer_id: "certik_auditor_01".to_string(),
        organization: "CertiK".to_string(),
        method: "formal_verification".to_string(),
        audit_trail_ref: Some(compute_sha256("certik_audit_trail")),
        secondary_source_ref: Some("https://certik.com/reports/certik_2026_101".to_string()),
    };

    let audit_text = "Independent benchmark audit performed by CertiK confirms 4x throughput improvement with formal verification across clusters.";
    let claim = ClaimEvidence {
        claim_id: "claim_01".to_string(),
        statement: "Verified 4x throughput confirmed by CertiK".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: "https://certik.com/reports/certik_2026_101".to_string(),
            excerpt: "4x throughput with formal verification".to_string(),
            locator: None,
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(rec.clone()),
    };

    // Case 1: Secondary resolver returns a general blog post / mention with record ID,
    // but without official audit report classification structure.
    let generic_mention = "CertiK mentions certik_2026_101 in an informal blog post discussion.";
    let res_generic = move |_r: &str| -> Option<String> { Some(generic_mention.to_string()) };
    let status_generic = check_audit_provenance_level_with_resolver(
        audit_text,
        &rec,
        &claim,
        Some(&res_generic),
    );
    // Must be classified as ResolvedDocument, NOT IssuerAuthenticated!
    assert_eq!(status_generic, AuditProvenanceStatus::ResolvedDocument);
    assert!(!is_audit_source_verified_with_resolver(
        audit_text,
        &rec,
        Some(&res_generic)
    ));

    // Case 2: Secondary resolver returns an official published audit report with formal verification structure.
    let official_report = "Official CertiK Benchmark Audit Report 2026-101. Security assessment and formal verification for certik_2026_101 passed.";
    let res_official = move |_r: &str| -> Option<String> { Some(official_report.to_string()) };
    let status_official = check_audit_provenance_level_with_resolver(
        audit_text,
        &rec,
        &claim,
        Some(&res_official),
    );
    // Must be IssuerAuthenticated!
    assert_eq!(status_official, AuditProvenanceStatus::IssuerAuthenticated);
    assert!(is_audit_source_verified_with_resolver(
        audit_text,
        &rec,
        Some(&res_official)
    ));
}

// ---------------------------------------------------------------------------
// 3. P1: OS-Level Advisory File Lock Concurrency & Drop Release
// ---------------------------------------------------------------------------
#[test]
fn test_kernel_advisory_file_lock_concurrency_and_release() {
    let tmp = make_temp_dir("kernel_lock");
    let lock_path = tmp.join(".state.lock");

    // 1. Acquire first lock
    let lock1 = ProcessFileLock::acquire(&lock_path).expect("First lock acquire must succeed");

    // 2. Second acquire from another instance on same path must block or timeout
    // Using a separate thread to attempt acquire with a short wait:
    let lock_path_clone = lock_path.clone();
    let thread = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        // Since timeout is 5 seconds in ProcessFileLock, we expect it to take time while locked
        let res = ProcessFileLock::acquire(&lock_path_clone);
        (res.is_ok(), start.elapsed())
    });

    // Give child thread time to start and see lock held
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 3. Explicitly drop lock1 to release advisory flock
    drop(lock1);

    // 4. Thread should now acquire successfully
    let (acquired, elapsed) = thread.join().expect("Thread must finish");
    assert!(acquired, "Second lock should succeed after lock1 drops");
    assert!(elapsed >= std::time::Duration::from_millis(100));
}

// ---------------------------------------------------------------------------
// 4. P1: Transactional Handoff Commit & State Reconciliation Across Restarts
// ---------------------------------------------------------------------------
#[test]
fn test_transactional_commit_and_state_reconciliation_recovery() {
    let tmp = make_temp_dir("commit_reconciliation");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));

    let candidate = NormalizedCandidate {
        candidate_id: "cand_trans_01".to_string(),
        title: "Transactional Consistency Test".to_string(),
        published_at_utc: current_utc_timestamp(),
        ..Default::default()
    };
    storage.upsert_candidates(&[candidate]);

    // 1. Reserve handoff
    let token = storage
        .reserve_handoff("cand_trans_01", 300, "worker_alpha")
        .expect("Reservation must succeed");
    assert!(!token.is_empty());

    // 2. Commit handoff to PUBLISHED
    let committed = storage
        .commit_consumed("cand_trans_01", &token)
        .expect("Commit must succeed");
    assert!(committed);
    assert!(storage.is_consumed("cand_trans_01"));

    // 3. Simulate fault: consumed.json is wiped or corrupt, but reservations.json is intact
    let consumed_file = tmp.join(".state").join("consumed.json");
    let _ = std::fs::remove_file(&consumed_file);
    assert!(!consumed_file.exists());

    // 4. Restart storage instance from disk
    let recovered_storage = FileStorageAdapter::new(&tmp);

    // 5. State reconciliation on startup MUST recover cand_trans_01 as consumed!
    assert!(
        recovered_storage.is_consumed("cand_trans_01"),
        "State reconciliation must recognize PUBLISHED reservation as consumed on restart"
    );
}

// ---------------------------------------------------------------------------
// 5. P1: Cached Readiness Revalidation In-Place Persistence Across Restarts
// ---------------------------------------------------------------------------
#[test]
fn test_cached_readiness_downgrade_persists_across_restarts() {
    let tmp = make_temp_dir("cached_revalidation_persist");
    let state_dir = tmp.join(".state");
    let evidence_dir = state_dir.join("evidence");
    std::fs::create_dir_all(&evidence_dir).unwrap();

    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let scorer = CandidateScorer::default();
    let fetcher = Arc::new(MockFeedFetcher { items: vec![] });
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer);

    let primary_text = "Authentic primary source text for benchmark.";
    let primary_sha = compute_sha256(primary_text);
    std::fs::write(evidence_dir.join(format!("{primary_sha}.txt")), primary_text).unwrap();

    let cand = NormalizedCandidate {
        candidate_id: "cand_persist_01".to_string(),
        title: "Benchmark Story".to_string(),
        published_at_utc: current_utc_timestamp(),
        content_origin: ContentOrigin::Article,
        research: ResearchEvidenceFields {
            primary_source: Some(PrimarySourceEvidence {
                url: "https://example.com/bench".to_string(),
                fetch_status: "SUCCESS".to_string(),
                content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
                snapshot_ref: Some(format!("https://example.com/evidence/{primary_sha}.txt")),
                snapshot_sha256: Some(primary_sha.clone()),
                ..Default::default()
            }),
            research_readiness: Some(daily_news::domain::ResearchReadiness {
                status: ReadinessStatus::VideoReady,
                reasons: vec!["Previously verified".to_string()],
                missing_fields: vec![],
                contract_version: "2.0".to_string(),
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    // Save as VideoReady initially
    storage.save_candidates(&[cand.clone()]).unwrap();

    // Now corrupt the primary snapshot text on disk (simulating evidence alteration or corruption)
    std::fs::write(evidence_dir.join(format!("{primary_sha}.txt")), "CORRUPTED CONTENT").unwrap();

    // Call get_candidates: reverify_cached_readiness_if_needed must detect corruption and downgrade
    let retrieved = service.get_candidates(10);
    let found = retrieved
        .iter()
        .find(|c| c.candidate_id == "cand_persist_01")
        .expect("Candidate must be retrieved");
    let readiness_after = found.research.research_readiness.as_ref().unwrap();
    assert_eq!(
        readiness_after.status,
        ReadinessStatus::NeedsRevalidation,
        "Readiness must be downgraded to NeedsRevalidation due to snapshot checksum mismatch"
    );

    // Restart storage and service from disk (retrieve -> downgrade -> persist -> restart -> retrieve)
    let restarted_storage = Arc::new(FileStorageAdapter::new(&tmp));
    let restarted_fetcher = Arc::new(MockFeedFetcher { items: vec![] });
    let _restarted_service = DailyNewsService::new(restarted_fetcher, restarted_storage.clone(), CandidateScorer::default());

    let reloaded_candidates = restarted_storage.get_cached_candidates();
    let reloaded_cand = reloaded_candidates
        .iter()
        .find(|c| c.candidate_id == "cand_persist_01")
        .expect("Candidate must persist on disk");
    assert_eq!(
        reloaded_cand.research.research_readiness.as_ref().unwrap().status,
        ReadinessStatus::NeedsRevalidation,
        "Downgraded readiness MUST be durably persisted to disk and survive restarts"
    );
}

// ---------------------------------------------------------------------------
// 6. P1: MCP Tools Schema Exposure for Handoff Lifecycle
// ---------------------------------------------------------------------------
#[test]
fn test_mcp_tools_schema_includes_full_handoff_lifecycle() {
    let tools = daily_news::mcp_schema::get_mcp_tools_list();
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();

    assert!(
        tool_names.contains(&"verify_production_handoff"),
        "MCP schema must declare verify_production_handoff"
    );
    assert!(
        tool_names.contains(&"release_production_handoff"),
        "MCP schema must declare release_production_handoff"
    );
    assert!(
        tool_names.contains(&"commit_production_handoff"),
        "MCP schema must declare commit_production_handoff"
    );

    // Verify required fields in schemas
    let release_tool = tools
        .iter()
        .find(|t| t["name"] == "release_production_handoff")
        .unwrap();
    let release_req: Vec<&str> = release_tool["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(release_req.contains(&"candidate_id"));
    assert!(release_req.contains(&"token"));

    let commit_tool = tools
        .iter()
        .find(|t| t["name"] == "commit_production_handoff")
        .unwrap();
    let commit_req: Vec<&str> = commit_tool["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(commit_req.contains(&"candidate_id"));
    assert!(commit_req.contains(&"token"));
}
