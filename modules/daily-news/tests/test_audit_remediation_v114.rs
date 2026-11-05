use daily_news::domain::{
    ClaimEvidence, ContentOrigin, EvidenceRef, NormalizedCandidate, RawProvenance,
    ReadinessStatus, VerificationRecord, VisualEvidence,
};
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::research_readiness::{
    are_visuals_provenance_verified, is_valid_visual_asset_format,
    verify_claim_semantic_entailment, ClaimGroundingStatus,
};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::source_capture::HttpSourceCaptureAdapter;
use daily_news::storage::{resolve_state_dir, FileStorageAdapter};
use daily_news::url::{compute_sha256, compute_sha256_bytes};
use daily_news::StoragePort;
use serde_json::json;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dn_v114_{tag}_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// =========================================================================
// 1. P0: True Audit Resolver in Production Enrichment & Readiness Gate
// =========================================================================

#[test]
fn test_v114_production_audit_resolver_corroboration_gate() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral server");
    let port = listener.local_addr().unwrap().port();

    let server_body = "<html><head><title>Deterministic Async Engine</title></head><body><p>Our production deployment achieved a 4x reduction in cache latency across edge nodes.</p></body></html>";
    let body_clone = server_body.to_string();

    let _server_handle = std::thread::spawn(move || {
        for _ in 0..4 {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_clone.len(),
                    body_clone
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    });

    let tmp_dir = make_temp_dir("audit_gate");
    let storage = Arc::new(FileStorageAdapter::new(&tmp_dir));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp_dir).with_allow_http(true));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();

    let service = DailyNewsService::new(fetcher, storage.clone(), scorer)
        .with_source_capture(capturer);

    let cand_url = format!("http://127.0.0.1:{port}/article");
    let cand_id = "cand_v114_test";
    let cand = NormalizedCandidate {
        candidate_id: cand_id.to_string(),
        canonical_url: cand_url.clone(),
        original_url: cand_url.clone(),
        url: cand_url.clone(),
        source_url: cand_url.clone(),
        source_type: "news".to_string(),
        title: "Deterministic Async Engine".to_string(),
        publisher: "example.com".to_string(),
        normalized_publisher: "example".to_string(),
        published_at_utc: "2026-11-04T05:00:00Z".to_string(),
        published_at: "2026-11-04T05:00:00Z".to_string(),
        content_hash: "chash_v114".to_string(),
        title_hash: "thash_v114".to_string(),
        raw_provenance: RawProvenance::default(),
        adapter_version: "1.14.0".to_string(),
        topic_family: "tech".to_string(),
        author: None,
        description: None,
        body: None,
        summary: None,
        tags: vec!["tech".to_string()],
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
    storage.cache_candidates(&[cand]);

    // Initial capture
    let initial_res = service.enrich_candidate(cand_id, None, None, None).unwrap();
    let primary_sha = initial_res.candidate.primary_source.as_ref().unwrap().snapshot_sha256.clone().unwrap();

    let chart_bytes = b"PNG MOCK CHART ASSET DATA FOR V114 AUDIT TEST";
    let chart_sha = compute_sha256_bytes(chart_bytes);
    let evidence_dir = resolve_state_dir(&tmp_dir).join("evidence");
    let _ = std::fs::create_dir_all(&evidence_dir);
    std::fs::write(evidence_dir.join(format!("{chart_sha}.txt")), chart_bytes).unwrap();

    let visuals = vec![VisualEvidence {
        asset_ref: "vis_v114_chart".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/v114_chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(chart_sha),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    // 1. Non-existent audit trail file in storage fails closed to NeedsVerification
    let missing_audit_sha = "a".repeat(64);
    let claims_with_missing_audit = vec![ClaimEvidence {
        claim_id: "c_v114_1".to_string(),
        statement: "4x reduction in cache latency".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha.clone(),
            excerpt: "Our production deployment achieved a 4x reduction in cache latency across edge nodes.".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "vr_missing_01".to_string(),
            reviewer_id: "auditor_lab".to_string(),
            organization: "Benchmarking Laboratory".to_string(),
            method: "reproducible_benchmark".to_string(),
            verified_at: "2026-11-04T00:00:00Z".to_string(),
            audit_trail_ref: Some(missing_audit_sha),
            secondary_source_ref: None,
        }),
    }];

    let res_missing = service.enrich_candidate(cand_id, None, Some(claims_with_missing_audit), Some(visuals.clone())).unwrap();
    assert_eq!(res_missing.candidate.research_readiness.as_ref().unwrap().status, ReadinessStatus::NeedsVerification);

    // 2. Corroborated audit file in storage with mock provenance validates to MockVerified
    let audit_doc = "Official Audit Report #2026-V114: Benchmarking Laboratory independently verified the benchmark findings and confirmed 4x reduction in cache latency across reproducible test runs.";
    let audit_sha = compute_sha256(audit_doc);
    std::fs::write(evidence_dir.join(format!("{audit_sha}.txt")), audit_doc.as_bytes()).unwrap();

    let claims_with_valid_audit = vec![ClaimEvidence {
        claim_id: "c_v114_valid".to_string(),
        statement: "4x reduction in cache latency".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha.clone(),
            excerpt: "Our production deployment achieved a 4x reduction in cache latency across edge nodes.".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "vr_valid_01".to_string(),
            reviewer_id: "auditor_lab".to_string(),
            organization: "Benchmarking Laboratory".to_string(),
            method: "reproducible_benchmark".to_string(),
            verified_at: "2026-11-04T00:00:00Z".to_string(),
            audit_trail_ref: Some(audit_sha),
            secondary_source_ref: None,
        }),
    }];

    let res_valid = service.enrich_candidate(cand_id, None, Some(claims_with_valid_audit), Some(visuals)).unwrap();
    assert_eq!(res_valid.candidate.research_readiness.as_ref().unwrap().status, ReadinessStatus::MockVerified);
    assert!(!res_valid.production_scoring.is_video_production_ready);

    // 3. Trusted accredited issuer + real PNG bytes reaches production VideoReady
    let trusted_audit_doc = "Official CertiK Audit Report #2026-V114: CertiK independently verified the benchmark findings and confirmed 4x reduction in cache latency across reproducible test runs.";
    let trusted_audit_sha = compute_sha256(trusted_audit_doc);
    std::fs::write(evidence_dir.join(format!("{trusted_audit_sha}.txt")), trusted_audit_doc.as_bytes()).unwrap();

    let real_png_bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\x0bIDATx\x9cc`\0\x02\0\0\x05\0\x01z^\xab?\0\0\0\0IEND\xaeB`\x82";
    let real_png_sha = compute_sha256_bytes(real_png_bytes);
    std::fs::write(evidence_dir.join(format!("{real_png_sha}.txt")), real_png_bytes).unwrap();

    let trusted_visuals = vec![VisualEvidence {
        asset_ref: "vis_v114_chart_prod".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/v114_chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(real_png_sha),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    }];

    let claims_with_trusted_audit = vec![ClaimEvidence {
        claim_id: "c_v114_trusted".to_string(),
        statement: "4x reduction in cache latency".to_string(),
        evidence_refs: vec![EvidenceRef {
            source_ref: primary_sha,
            excerpt: "Our production deployment achieved a 4x reduction in cache latency across edge nodes.".to_string(),
            locator: Some("p:1".to_string()),
        }],
        verification_status: "INDEPENDENTLY_VERIFIED".to_string(),
        reviewer: None,
        verification_record: Some(VerificationRecord {
            record_id: "vr_trusted_01".to_string(),
            reviewer_id: "certik_lead_auditor".to_string(),
            organization: "CertiK".to_string(),
            method: "reproducible_benchmark".to_string(),
            verified_at: "2026-11-04T00:00:00Z".to_string(),
            audit_trail_ref: Some(trusted_audit_sha),
            secondary_source_ref: None,
        }),
    }];

    let res_trusted = service.enrich_candidate(cand_id, None, Some(claims_with_trusted_audit), Some(trusted_visuals)).unwrap();
    assert_eq!(res_trusted.candidate.research_readiness.as_ref().unwrap().status, ReadinessStatus::VideoReady);
    assert!(res_trusted.production_scoring.is_video_production_ready);
}

// =========================================================================
// 2. P0: HTML Snapshots Rejected by Visual Asset Format Gate
// =========================================================================

#[test]
fn test_v114_html_snapshot_cannot_bypass_visual_asset_gate() {
    let html_bytes = b"<!DOCTYPE html><html><body><h1>Article</h1><p>Not an image</p></body></html>";
    assert!(!is_valid_visual_asset_format(html_bytes));

    let php_bytes = b"<?php echo 'malicious'; ?>";
    assert!(!is_valid_visual_asset_format(php_bytes));

    let text_bytes = b"Just plain text description of a chart";
    assert!(!is_valid_visual_asset_format(text_bytes));

    // Valid visual formats
    let png_magic = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR...";
    assert!(is_valid_visual_asset_format(png_magic));

    let png_mock = b"PNG MOCK ASSET DATA";
    assert!(is_valid_visual_asset_format(png_mock));

    let jpeg_magic = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    assert!(is_valid_visual_asset_format(jpeg_magic));

    let gif_magic = b"GIF89a...";
    assert!(is_valid_visual_asset_format(gif_magic));

    let svg_data = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"10\"/></svg>";
    assert!(is_valid_visual_asset_format(svg_data));

    // Passing HTML snapshot to visual evidence resolver must fail closed
    let html_sha = compute_sha256_bytes(html_bytes);
    let visual = VisualEvidence {
        asset_ref: "vis_html_bypass".to_string(),
        kind: "chart".to_string(),
        source_url: "https://example.com/chart.png".to_string(),
        source_snapshot_ref: None,
        license_status: Some("cc-by".to_string()),
        provenance_status: Some("verified".to_string()),
        checksum_sha256: Some(html_sha.clone()),
        license_evidence_ref: Some("https://example.com/license".to_string()),
        verified_rights: Some(true),
    };

    let resolver = move |sha: &str| -> Option<Vec<u8>> {
        if sha == html_sha {
            Some(html_bytes.to_vec())
        } else {
            None
        }
    };

    assert!(!are_visuals_provenance_verified(&[visual], Some(&resolver)));
}

// =========================================================================
// 3. P1: Consistency of State Directory Resolution
// =========================================================================

#[test]
fn test_v114_resolve_state_dir_consistency() {
    let tmp = make_temp_dir("state_dir");
    let base = &tmp;

    // Standard writable base resolves to base/.state
    let resolved = resolve_state_dir(base);
    assert_eq!(resolved, base.join(".state"));

    // Environment variable override
    let override_dir = base.join("custom_state");
    std::env::set_var("DAILY_NEWS_STATE_DIR", override_dir.to_str().unwrap());
    let resolved_override = resolve_state_dir(base);
    assert_eq!(resolved_override, override_dir);
    std::env::remove_var("DAILY_NEWS_STATE_DIR");
}

// =========================================================================
// 4. P1: Semantic Entailment Precision (No Coincidental False Positives)
// =========================================================================

#[test]
fn test_v114_semantic_entailment_eliminates_two_word_override_false_positives() {
    // A long statement with 10 informative words where only 2 words happen to match
    let statement = "Distributed consensus algorithm achieved zero message loss under catastrophic network partitions";
    // Excerpt only mentions "distributed" and "algorithm" in a completely unrelated context
    let unrelated_excerpt = "We introduced a distributed pipeline algorithm for video decoding.";

    let res = verify_claim_semantic_entailment(statement, unrelated_excerpt);
    assert_eq!(res, ClaimGroundingStatus::ExcerptFound, "Low ratio match in long statement must NOT entail");

    // Positive case: High ratio match (>50%) with substantive tokens
    let substantive_statement = "Zero message loss under catastrophic network partitions";
    let corroborated_excerpt = "Our stress testing verified zero message loss under catastrophic network partitions.";
    let res_pos = verify_claim_semantic_entailment(substantive_statement, corroborated_excerpt);
    assert_eq!(res_pos, ClaimGroundingStatus::ClaimEntailed);
}

// =========================================================================
// 5. P1: ID and Config Error Validation in Handlers
// =========================================================================

#[test]
fn test_v114_consume_and_config_validation() {
    let tmp = make_temp_dir("handlers");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();

    let service = DailyNewsService::new(fetcher, storage, scorer)
        .with_source_capture(capturer);

    // 1. handle_consume with empty parameters returns 400
    let resp_empty = daily_news::handlers::handle_consume(&service, &json!({}));
    assert_eq!(resp_empty["status"], 400);

    let resp_empty_arr = daily_news::handlers::handle_consume(&service, &json!({ "candidate_ids": [] }));
    assert_eq!(resp_empty_arr["status"], 400);

    let resp_whitespace = daily_news::handlers::handle_consume(&service, &json!({ "candidate_id": "   " }));
    assert_eq!(resp_whitespace["status"], 400);

    // 2. handle_config validation returns 400 on invalid types or bounds
    let resp_invalid_type = daily_news::handlers::handle_config(&service, "POST", &json!({ "freshness_window_hours": "invalid_string" }));
    assert_eq!(resp_invalid_type["status"], 400);

    let resp_invalid_range = daily_news::handlers::handle_config(&service, "POST", &json!({ "freshness_window_hours": -5.0 }));
    assert_eq!(resp_invalid_range["status"], 400);

    let resp_not_object = daily_news::handlers::handle_config(&service, "POST", &json!(["array_not_object"]));
    assert_eq!(resp_not_object["status"], 400);

    // 3. Valid config update returns 200
    let resp_valid = daily_news::handlers::handle_config(&service, "POST", &json!({ "freshness_window_hours": 36.0 }));
    assert_eq!(resp_valid["status"], 200);
}

// =========================================================================
// 6. P2: MCP Conformance for Modern Protocol Methods & Notifications
// =========================================================================

#[test]
fn test_v114_mcp_modern_protocol_conformance() {
    let tmp = make_temp_dir("mcp_conf");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let capturer = Arc::new(HttpSourceCaptureAdapter::new(&tmp));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();

    let service = DailyNewsService::new(fetcher, storage, scorer)
        .with_source_capture(capturer);

    // initialize capabilities
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let init_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &init_req);
    assert_eq!(init_resp["status"], 200);
    let init_json: serde_json::Value = serde_json::from_str(init_resp["body"].as_str().unwrap()).unwrap();
    let caps = &init_json["result"]["capabilities"];
    assert!(caps.get("tools").is_some());
    assert!(caps.get("resources").is_some());
    assert!(caps.get("prompts").is_some());
    assert!(caps.get("logging").is_some());

    // prompts/list
    let prompts_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "prompts/list"
    });
    let prompts_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &prompts_req);
    assert_eq!(prompts_resp["status"], 200);
    let prompts_json: serde_json::Value = serde_json::from_str(prompts_resp["body"].as_str().unwrap()).unwrap();
    assert!(prompts_json["result"]["prompts"].is_array());

    // resources/templates/list
    let templates_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "resources/templates/list"
    });
    let templates_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &templates_req);
    assert_eq!(templates_resp["status"], 200);
    let templates_json: serde_json::Value = serde_json::from_str(templates_resp["body"].as_str().unwrap()).unwrap();
    assert!(templates_json["result"]["resourceTemplates"].is_array());

    // logging/setLevel
    let log_req = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "logging/setLevel",
        "params": { "level": "info" }
    });
    let log_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &log_req);
    assert_eq!(log_resp["status"], 200);

    // roots/list
    let roots_req = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "roots/list"
    });
    let roots_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &roots_req);
    assert_eq!(roots_resp["status"], 200);
    let roots_json: serde_json::Value = serde_json::from_str(roots_resp["body"].as_str().unwrap()).unwrap();
    assert!(roots_json["result"]["roots"].is_array());

    // Notifications return 204 No Content
    let notif_init = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let notif_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &notif_init);
    assert_eq!(notif_resp["status"], 204);

    let notif_progress = json!({
        "jsonrpc": "2.0",
        "method": "notifications/progress",
        "params": { "progress": 50 }
    });
    let notif_prog_resp = daily_news::mcp::handle_jsonrpc_mcp_request(&service, &tmp, &notif_progress);
    assert_eq!(notif_prog_resp["status"], 204);
}
