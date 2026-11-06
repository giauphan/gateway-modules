//! Integration tests for native Model Context Protocol (MCP) tool support in daily-news.
//!
//! Enforces Clean Architecture & DDD interface adapter rules:
//! - Validates that `/tools/call` dispatches to proper domain use cases.
//! - Verifies fail-closed input sanitization for all tool parameters.
//! - Checks standardized MCP content structures and error envelopes.

use daily_news::mcp::{get_mcp_tools_list, handle_mcp_tool_request};
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::FileStorageAdapter;
use daily_news::{NormalizedCandidate, RawNewsItem, StoragePort};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

struct MockFeedFetcher;
impl daily_news::ports::FeedFetcherPort for MockFeedFetcher {
    fn fetch_raw_items(&self, _limit: usize) -> Result<Vec<RawNewsItem>, String> {
        let mut item1 = RawNewsItem::new(
            "hn_mcp_1",
            "https://example.com/rust-mcp-engine",
            "Rust Native MCP Engine Released",
            "2026-11-04T10:00:00Z",
        );
        item1.tags = vec!["rust".to_string(), "mcp".to_string()];

        let mut item2 = RawNewsItem::new(
            "hn_mcp_2",
            "https://example.com/ai-agent-protocols",
            "Autonomous Agent Protocols Standardized",
            "2026-11-04T09:00:00Z",
        );
        item2.tags = vec!["ai".to_string()];

        Ok(vec![item1, item2])
    }
}

struct MockSourceCapture {
    snapshot_text: String,
}

impl daily_news::ports::SourceCapturePort for MockSourceCapture {
    fn capture_source(
        &self,
        url: &str,
    ) -> Result<daily_news::domain::PrimarySourceEvidence, String> {
        let sha256 = daily_news::url::compute_sha256(&self.snapshot_text);
        Ok(daily_news::domain::PrimarySourceEvidence {
            url: url.to_string(),
            fetch_status: "SUCCESS".to_string(),
            content_quality: Some("ARTICLE_CONTENT_VALID".to_string()),
            content_type: Some("text/html".to_string()),
            title: Some("Captured Title".to_string()),
            published_at: Some("2026-11-04T10:00:00Z".to_string()),
            excerpt: Some(self.snapshot_text.clone()),
            snapshot_ref: Some(format!(".state/evidence/{sha256}.txt")),
            snapshot_sha256: Some(sha256),
            captured_at: Some("2026-11-04T10:00:00Z".to_string()),
            extraction_method: Some("mock".to_string()),
        })
    }

    fn read_snapshot_text(&self, _snapshot_ref: &str) -> Option<String> {
        Some(self.snapshot_text.clone())
    }
}

fn create_test_service(base_dir: &PathBuf) -> (DailyNewsService, Arc<FileStorageAdapter>) {
    let storage = Arc::new(FileStorageAdapter::new(base_dir));
    let mock_capture = Arc::new(MockSourceCapture {
        snapshot_text: "Claims verified through MCP interface and grounded in snapshot text."
            .to_string(),
    });
    let service = DailyNewsService::new(
        Arc::new(MockFeedFetcher),
        storage.clone(),
        CandidateScorer::default(),
    )
    .with_source_capture(mock_capture);
    (service, storage)
}

#[test]
fn test_mcp_tools_list_schema() {
    let tools = get_mcp_tools_list();
    assert_eq!(tools.len(), 9, "Must expose declared MCP tools including production handoff lifecycle");

    let expected_names = [
        "discover_candidates",
        "get_candidate",
        "enrich_candidate",
        "verify_research_readiness",
        "verify_production_handoff",
        "release_production_handoff",
        "commit_production_handoff",
        "get_evidence_snapshot",
        "get_discussion",
    ];

    for name in &expected_names {
        assert!(
            tools.iter().any(|t| t["name"].as_str() == Some(name)),
            "Tool '{name}' must be present in declared toolset"
        );
    }
}

#[test]
fn test_mcp_discover_candidates_tool() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_disc_{}", uuid::Uuid::new_v4()));
    let (service, _) = create_test_service(&tmp);

    let req_body = json!({
        "name": "daily-news__discover_candidates",
        "arguments": {
            "limit": 5,
            "topics": ["rust"]
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    assert_eq!(parsed["is_error"], Value::Null);

    let content_arr = parsed["content"].as_array().expect("content must be array");
    assert_eq!(content_arr.len(), 1);
    assert_eq!(content_arr[0]["type"], "text");

    let text_val: Value = serde_json::from_str(content_arr[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["status"], "ELIGIBLE");

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_get_candidate_tool() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_get_{}", uuid::Uuid::new_v4()));
    let (service, storage) = create_test_service(&tmp);

    let cand = NormalizedCandidate {
        candidate_id: "cand_test_mcp_001".to_string(),
        title: "Test MCP Candidate".to_string(),
        canonical_url: "https://example.com/test-mcp".to_string(),
        ..Default::default()
    };
    storage.upsert_candidates(&[cand]);

    let req_body = json!({
        "name": "get_candidate",
        "arguments": {
            "candidate_id": "cand_test_mcp_001"
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    let text_val: Value =
        serde_json::from_str(parsed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["candidate_id"], "cand_test_mcp_001");
    assert_eq!(text_val["title"], "Test MCP Candidate");

    // Missing candidate returns error
    let missing_body = json!({
        "name": "get_candidate",
        "arguments": { "candidate_id": "cand_nonexistent" }
    });
    let missing_resp = handle_mcp_tool_request(&service, &tmp, &missing_body, None);
    assert_eq!(missing_resp["status"], 400);

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_enrich_candidate_tool() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_enrich_{}", uuid::Uuid::new_v4()));
    let (service, storage) = create_test_service(&tmp);

    let cand = NormalizedCandidate {
        candidate_id: "cand_enrich_test".to_string(),
        title: "Enrich Candidate Test".to_string(),
        canonical_url: "https://example.com/enrich-test".to_string(),
        ..Default::default()
    };
    storage.upsert_candidates(&[cand]);

    let req_body = json!({
        "name": "enrich_candidate",
        "arguments": {
            "candidate_id": "cand_enrich_test",
            "claims": [
                {
                    "claim_id": "claim_01",
                    "statement": "Claims verified through MCP interface",
                    "verification_status": "SOURCE_SUPPORTED",
                    "evidence_refs": []
                }
            ]
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    let text_val: Value =
        serde_json::from_str(parsed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["candidate"]["candidate_id"], "cand_enrich_test");

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_verify_research_readiness_tool() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_readiness_{}", uuid::Uuid::new_v4()));
    let (service, storage) = create_test_service(&tmp);

    let cand = NormalizedCandidate {
        candidate_id: "cand_ready_test".to_string(),
        title: "Ready Test".to_string(),
        canonical_url: "https://example.com/ready".to_string(),
        ..Default::default()
    };
    storage.upsert_candidates(&[cand]);

    let req_body = json!({
        "name": "verify_research_readiness",
        "arguments": {
            "candidate_id": "cand_ready_test"
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    let text_val: Value =
        serde_json::from_str(parsed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["status"], "NEEDS_SOURCE");

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_get_evidence_snapshot_tool() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_snap_{}", uuid::Uuid::new_v4()));
    let (service, _) = create_test_service(&tmp);

    let evidence_dir = tmp.join(".state").join("evidence");
    let _ = fs::create_dir_all(&evidence_dir);

    let test_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let test_file = evidence_dir.join(format!("{test_hash}.txt"));
    fs::write(
        &test_file,
        "Verified snapshot artifact content for MCP testing.",
    )
    .unwrap();

    let req_body = json!({
        "name": "get_evidence_snapshot",
        "arguments": {
            "sha256": test_hash
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    let text_val: Value =
        serde_json::from_str(parsed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["sha256"], test_hash);
    assert_eq!(
        text_val["content"],
        "Verified snapshot artifact content for MCP testing."
    );

    // Invalid hash format fails closed
    let invalid_body = json!({
        "name": "get_evidence_snapshot",
        "arguments": { "sha256": "invalid_too_short" }
    });
    let inv_resp = handle_mcp_tool_request(&service, &tmp, &invalid_body, None);
    assert_eq!(inv_resp["status"], 400);

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_unknown_tool_fails_closed() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_unknown_{}", uuid::Uuid::new_v4()));
    let (service, _) = create_test_service(&tmp);

    let req_body = json!({
        "name": "daily-news__non_existent_tool",
        "arguments": {}
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 400);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    assert_eq!(parsed["is_error"], true);
    assert!(parsed["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Unknown or unsupported MCP tool"));

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_get_discussion_tool() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_disc_tool_{}", uuid::Uuid::new_v4()));
    let (mut service, _) = create_test_service(&tmp);

    struct MockDiscussion;
    impl daily_news::ports::DiscussionFetcherPort for MockDiscussion {
        fn fetch_thread(
            &self,
            url_or_id: &str,
            _limit: Option<usize>,
        ) -> Result<daily_news::domain::DiscussionThread, String> {
            Ok(daily_news::domain::DiscussionThread {
                id: url_or_id.to_string(),
                subreddit: None,
                title: Some("Mock Discussion Title".to_string()),
                author: Some("mock_author".to_string()),
                permalink: format!("https://news.ycombinator.com/item?id={url_or_id}"),
                url: Some(format!("https://news.ycombinator.com/item?id={url_or_id}")),
                selftext: None,
                score: 10,
                num_comments: 5,
                comments: vec![],
            })
        }
    }

    service = service.with_discussion_fetcher(Arc::new(MockDiscussion));

    let req_body = json!({
        "name": "get_discussion",
        "arguments": {
            "url_or_id": "12345"
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, None);
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    let text_val: Value =
        serde_json::from_str(parsed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["id"], "12345");

    // Missing url_or_id returns error
    let req_missing = json!({
        "name": "get_discussion",
        "arguments": {}
    });
    let resp_missing = handle_mcp_tool_request(&service, &tmp, &req_missing, None);
    assert_eq!(resp_missing["status"], 400);

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_mcp_header_tool_dispatch() {
    let tmp = std::env::temp_dir().join(format!("test_mcp_hdr_{}", uuid::Uuid::new_v4()));
    let (service, storage) = create_test_service(&tmp);

    let cand = NormalizedCandidate {
        candidate_id: "cand_header_test".to_string(),
        title: "Header Tool Dispatch Test".to_string(),
        canonical_url: "https://example.com/header-test".to_string(),
        ..Default::default()
    };
    storage.upsert_candidates(&[cand]);

    let req_body = json!({
        "arguments": {
            "candidate_id": "cand_header_test"
        }
    });

    let resp_val = handle_mcp_tool_request(&service, &tmp, &req_body, Some("get_candidate"));
    assert_eq!(resp_val["status"], 200);

    let body_str = resp_val["body"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(body_str).unwrap();
    let text_val: Value =
        serde_json::from_str(parsed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text_val["candidate_id"], "cand_header_test");

    let _ = fs::remove_dir_all(tmp);
}
