use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn send_rpc_request(req: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_daily-news"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to start daily-news binary");

    let req_line = format!("{}\n", serde_json::to_string(&req).unwrap());
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(req_line.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("Failed to read output");
    let out_str = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let first_line = out_str.lines().next().expect("No output line received");
    serde_json::from_str(first_line).expect("Invalid JSON response")
}

#[test]
fn test_adapter_health_endpoint() {
    let req = json!({
        "path": "/health",
        "method": "GET"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);

    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["module"], "daily-news");
    assert_eq!(body["engine"], "rust-native");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["reddit_enabled"], true);
}

#[test]
#[cfg(target_os = "linux")]
fn test_precompiled_release_binary_matches_version() {
    let precompiled_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("daily-news");
    assert!(
        precompiled_path.exists(),
        "Precompiled release binary must exist at {:?}",
        precompiled_path
    );
    let mut child = Command::new(&precompiled_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn precompiled binary");

    let req = json!({ "path": "/health", "method": "GET" });
    let req_line = format!("{}\n", serde_json::to_string(&req).unwrap());
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(req_line.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("Failed to read output");
    let out_str = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let first_line = out_str.lines().next().expect("No output line");
    let resp: Value = serde_json::from_str(first_line).expect("Invalid JSON");
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["engine"], "rust-native");
}

#[test]
fn test_adapter_models_endpoint() {
    let req = json!({
        "path": "/models",
        "method": "GET"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);

    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["id"], "daily-news");
}

#[test]
fn test_adapter_ui_endpoint() {
    let req = json!({
        "path": "/ui",
        "method": "GET"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);
    assert!(resp["headers"]["content-type"]
        .as_str()
        .unwrap()
        .contains("text/html"));
    assert!(resp["body"].as_str().unwrap().contains("Daily News"));
}

#[test]
fn test_adapter_flowchart_endpoint() {
    for path in ["/flow", "/flowchart"] {
        let req = json!({
            "path": path,
            "method": "GET"
        });
        let resp = send_rpc_request(req);
        assert_eq!(resp["status"], 200, "path {} should return 200", path);
        assert!(
            resp["headers"]["content-type"]
                .as_str()
                .unwrap()
                .contains("text/html"),
            "path {} should return text/html",
            path
        );
        let body = resp["body"].as_str().unwrap();
        assert!(
            body.contains("Daily News Architecture Explorer"),
            "path {} missing title",
            path
        );
        assert!(body.contains("<svg"), "path {} missing svg", path);
        assert!(
            body.contains("Clean Architecture"),
            "path {} missing clean arch tab",
            path
        );
    }
}

#[test]
fn test_adapter_candidates_custom_items() {
    let now = daily_news::date::current_utc_timestamp();
    let raw_items = json!([
        {
            "id": "item-1",
            "url": "https://docs.anthropic.com/agent-tools",
            "title": "Anthropic Enhances Tool Use in Claude",
            "publisher": "Anthropic",
            "published_at": now,
            "body": "Detailed guide on tool invocation with schema enforcement."
        }
    ]);

    let req = json!({
        "path": "/api/v1/candidates",
        "method": "POST",
        "body": serde_json::to_string(&json!({
            "items": raw_items,
            "limit": 5
        })).unwrap()
    });

    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);

    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["count"], 1);
    assert_eq!(
        body["candidates"][0]["title"],
        "Anthropic Enhances Tool Use in Claude"
    );
}

#[test]
fn test_adapter_topic_and_limit_aliases() {
    let curated = json!([
        {
            "id": "c_tool",
            "url": "https://mcp.io/tool-spec",
            "title": "Model Context Protocol Tool Schema Spec",
            "publisher": "MCP",
            "published_at": "2026-11-04T10:00:00+00:00",
            "body": "Tool call schemas and function parameters."
        },
        {
            "id": "c_debug",
            "url": "https://debug.io/trace",
            "title": "Debugging Flaky Test Retries in CI",
            "publisher": "DebugIO",
            "published_at": "2026-11-04T10:00:00+00:00",
            "body": "Crash traces and retry loops."
        }
    ]);

    // 1. Singular topic alias: ?topic=agent_tool_use
    let req_single = json!({
        "path": "/api/v1/discover?topic=agent_tool_use&count=1",
        "method": "GET",
        "body": serde_json::to_string(&json!({
            "curated_candidates": curated
        })).unwrap()
    });
    let resp1 = send_rpc_request(req_single);
    assert_eq!(resp1["status"], 200);
    let body1: Value = serde_json::from_str(resp1["body"].as_str().unwrap()).unwrap();
    assert_eq!(body1["status"], "ELIGIBLE");
    assert_eq!(body1["count"], 1);
    assert_eq!(body1["candidates"][0]["topic_family"], "agent_tool_use");

    // 2. Inverted date range validation returns 400
    let req_inv = json!({
        "path": "/api/v1/discover?from_date=2026-11-05&to_date=2026-11-01",
        "method": "GET"
    });
    let resp_inv = send_rpc_request(req_inv);
    assert_eq!(resp_inv["status"], 400);
    let body_inv: Value = serde_json::from_str(resp_inv["body"].as_str().unwrap()).unwrap();
    assert_eq!(body_inv["status"], "INVALID_DATE_RANGE");
}

#[test]
fn test_adapter_chat_completions() {
    let req = json!({
        "path": "/chat/completions",
        "method": "POST",
        "body": serde_json::to_string(&json!({ "limit": 2 })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("Daily News Briefing"));
}

#[test]
fn test_adapter_reddit_feed_endpoint() {
    let req = json!({
        "path": "/api/v1/feed",
        "method": "POST",
        "body": serde_json::to_string(&json!({ "sources": ["reddit"], "limit": 3 })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert!(body["items"].is_array());
    let count = body["count"].as_u64().unwrap() as usize;
    assert!(count <= 3);
    assert_eq!(body["items"].as_array().unwrap().len(), count);
}

#[test]
fn test_adapter_feed_endpoint_limit_enforcement() {
    // 1. Gateway route with query limit=25 (matches user report)
    let req = json!({
        "path": "/custom/daily-news/api/v1/feed?limit=25",
        "method": "GET"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    let count = body["count"].as_u64().unwrap() as usize;
    assert!(count <= 25, "Expected count <= 25, got {count}");
    assert_eq!(body["items"].as_array().unwrap().len(), count);

    // 2. Gateway route with low query limit=2
    let req_small = json!({
        "path": "/custom/daily-news/api/v1/feed?limit=2",
        "method": "GET"
    });
    let resp_small = send_rpc_request(req_small);
    assert_eq!(resp_small["status"], 200);
    let body_small: Value = serde_json::from_str(resp_small["body"].as_str().unwrap()).unwrap();
    let count_small = body_small["count"].as_u64().unwrap() as usize;
    assert!(count_small <= 2, "Expected count <= 2, got {count_small}");
    assert_eq!(body_small["items"].as_array().unwrap().len(), count_small);

    // 3. Comma-separated sources in query
    let req_csv = json!({
        "path": "/custom/daily-news/api/v1/feed?sources=hackernews,reddit&limit=3",
        "method": "GET"
    });
    let resp_csv = send_rpc_request(req_csv);
    assert_eq!(resp_csv["status"], 200);
    let body_csv: Value = serde_json::from_str(resp_csv["body"].as_str().unwrap()).unwrap();
    let count_csv = body_csv["count"].as_u64().unwrap() as usize;
    assert!(count_csv <= 3, "Expected count <= 3, got {count_csv}");
    assert_eq!(body_csv["items"].as_array().unwrap().len(), count_csv);
}

#[test]
fn test_adapter_non_object_json_body_with_query_string_does_not_panic() {
    let req = json!({
        "path": "/api/v1/feed?limit=2",
        "method": "POST",
        "body": "[1, 2, 3]"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);
}

#[test]
fn test_adapter_discussion_endpoint_missing_params() {
    let req = json!({
        "path": "/api/v1/discussion",
        "method": "POST",
        "body": "{}"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 400);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("Missing required parameter"));
}

#[test]
fn test_adapter_discussion_endpoint_invalid_url() {
    let req = json!({
        "path": "/api/v1/discussion",
        "method": "POST",
        "body": serde_json::to_string(&json!({ "url": "https://example.com/not-reddit" })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 404);
}

#[test]
fn test_adapter_enrich_missing_params() {
    let req = json!({
        "path": "/api/v1/enrich",
        "method": "POST",
        "body": "{}"
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 400);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("Missing required parameter"));
}

#[test]
fn test_adapter_enrich_ssrf_rejected() {
    let req = json!({
        "path": "/api/v1/enrich",
        "method": "POST",
        "body": serde_json::to_string(&json!({ "url": "https://169.254.169.254/latest/meta-data" })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 400);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert!(body["error"].as_str().unwrap().contains("blocked"));
}

#[test]
fn test_adapter_enrich_empty_url_falls_back_to_source_url() {
    let req = json!({
        "path": "/api/v1/enrich",
        "method": "POST",
        "body": serde_json::to_string(&json!({
            "url": "   ",
            "source_url": "https://169.254.169.254/latest/meta-data"
        })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 400);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert!(body["error"].as_str().unwrap().contains("blocked"));
}

#[test]
fn test_adapter_evidence_invalid_hash() {
    let req = json!({
        "path": "/api/v1/evidence",
        "method": "POST",
        "body": serde_json::to_string(&json!({ "sha256": "invalid_hash" })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 400);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("Invalid or missing sha256"));
}

#[test]
fn test_adapter_evidence_not_found() {
    let nonexistent_hash = "a".repeat(64);
    let req = json!({
        "path": "/api/v1/evidence",
        "method": "POST",
        "body": serde_json::to_string(&json!({ "sha256": nonexistent_hash })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 404);
}

#[test]
fn test_adapter_enrich_candidate_endpoint_validation() {
    // Missing candidate_id -> 400
    let req = json!({
        "path": "/api/v1/enrich-candidate",
        "method": "POST",
        "body": serde_json::to_string(&json!({})).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 400);

    // Nonexistent candidate_id -> 400 with descriptive error
    let req2 = json!({
        "path": "/api/v1/enrich-candidate",
        "method": "POST",
        "body": serde_json::to_string(&json!({
            "candidate_id": "cand_nonexistent_12345"
        })).unwrap()
    });
    let resp2 = send_rpc_request(req2);
    assert_eq!(resp2["status"], 400);
    let body: Value = serde_json::from_str(resp2["body"].as_str().unwrap()).unwrap();
    assert!(body["error"].as_str().unwrap().contains("Candidate not found"));
}

#[test]
#[ignore = "requires live external internet access"]
fn test_adapter_discussion_endpoint_live() {
    let req = json!({
        "path": "/api/v1/discussion",
        "method": "POST",
        "body": serde_json::to_string(&json!({
            "url": "https://www.reddit.com/r/LocalLLaMA/comments/1xmitzx/alibabas_qwen_code_repo_just_leaked_qwen4max/",
            "limit": 5
        })).unwrap()
    });
    let resp = send_rpc_request(req);
    assert_eq!(resp["status"], 200);
    let body: Value = serde_json::from_str(resp["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["id"], "1xmitzx");
    assert!(body["comments"].is_array());
}
