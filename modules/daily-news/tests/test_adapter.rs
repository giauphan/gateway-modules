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
fn test_adapter_candidates_custom_items() {
    let raw_items = json!([
        {
            "id": "item-1",
            "url": "https://docs.anthropic.com/agent-tools",
            "title": "Anthropic Enhances Tool Use in Claude",
            "publisher": "Anthropic",
            "published_at": "2026-11-02T10:00:00+00:00",
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
            "published_at": "2026-11-02T10:00:00+00:00",
            "body": "Tool call schemas and function parameters."
        },
        {
            "id": "c_debug",
            "url": "https://debug.io/trace",
            "title": "Debugging Flaky Test Retries in CI",
            "publisher": "DebugIO",
            "published_at": "2026-11-02T10:00:00+00:00",
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
