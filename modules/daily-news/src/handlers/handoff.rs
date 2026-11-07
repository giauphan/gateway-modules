//! Production readiness, handoff, and publish idempotency route handlers.

use crate::rpc::make_json_resp;
use crate::service::DailyNewsService;
use serde_json::{json, Value};
use std::path::Path;

pub fn handle_verify_research_readiness(
    service: &DailyNewsService,
    base_dir: &Path,
    body: &Value,
) -> Value {
    let cid = match body["candidate_id"].as_str() {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => {
            return make_json_resp(
                400,
                json!({ "error": "Missing required field: 'candidate_id'" }),
            );
        }
    };
    let state_dir = crate::storage::file::resolve_state_dir(base_dir);
    let evidence_dir = state_dir.join("evidence");
    let resolver = |evidence_ref: &str| -> Option<String> {
        let hash = evidence_ref
            .strip_prefix("snapshot://")
            .unwrap_or(evidence_ref);
        let file_path = evidence_dir.join(format!("{hash}.txt"));
        std::fs::read_to_string(file_path).ok()
    };
    match service.verify_research_readiness(cid, Some(&resolver)) {
        Ok(readiness) => make_json_resp(200, json!(readiness)),
        Err(e) => {
            let status = if e.contains("not found") { 404 } else { 500 };
            make_json_resp(status, json!({ "error": e }))
        }
    }
}

pub fn handle_handoff(service: &DailyNewsService, body: &Value) -> Value {
    let action = body["action"].as_str().unwrap_or("reserve");
    let cid = body["candidate_id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .map(str::trim)
        .unwrap_or("");
    if cid.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'candidate_id'" }),
        );
    }

    if action == "release" {
        let token = body["token"]
            .as_str()
            .or_else(|| body["reservation_token"].as_str())
            .unwrap_or("");
        if token.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required parameter: 'token'" }),
            );
        }
        return match service.release_handoff(cid, token) {
            Ok(true) => make_json_resp(200, json!({ "status": "RELEASED", "candidate_id": cid })),
            Ok(false) => make_json_resp(404, json!({ "status": "NOT_FOUND", "candidate_id": cid })),
            Err(e) => make_json_resp(422, json!({ "status": "REJECTED", "error": e })),
        };
    }

    if action == "commit" {
        let token = body["token"]
            .as_str()
            .or_else(|| body["reservation_token"].as_str())
            .unwrap_or("");
        if token.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required parameter: 'token'" }),
            );
        }
        let idemp_key = body["idempotency_key"]
            .as_str()
            .or_else(|| body["idempotencyKey"].as_str());
        return match service.commit_handoff_with_idempotency(cid, token, idemp_key) {
            Ok(true) => make_json_resp(200, json!({ "status": "PUBLISHED", "candidate_id": cid })),
            Ok(false) => make_json_resp(404, json!({ "status": "NOT_FOUND", "candidate_id": cid })),
            Err(e) => make_json_resp(422, json!({ "status": "REJECTED", "error": e })),
        };
    }

    match service.verify_production_handoff(cid) {
        Ok(handoff) => make_json_resp(
            200,
            json!({
                "status": "APPROVED",
                "candidate_id": cid,
                "readiness": handoff.candidate.research.research_readiness.as_ref().map(|r| r.status.as_str()).unwrap_or("UNKNOWN"),
                "candidate": handoff.candidate,
                "reservation_token": handoff.reservation_token,
                "lease_seconds": handoff.lease_seconds,
                "expires_at": handoff.expires_at,
            }),
        ),
        Err((code, msg)) => make_json_resp(
            422,
            json!({
                "status": "REJECTED",
                "code": code,
                "error": msg,
            }),
        ),
    }
}

pub fn handle_verify_production_handoff(service: &DailyNewsService, body: &Value) -> Value {
    let cid = match body["candidate_id"].as_str() {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => {
            return make_json_resp(
                400,
                json!({ "error": "Missing required field: 'candidate_id'" }),
            );
        }
    };
    match service.verify_production_handoff(cid) {
        Ok(handoff) => make_json_resp(
            200,
            json!({
                "status": "VERIFIED",
                "handoff": handoff
            }),
        ),
        Err((code, err)) => {
            let status_code = match code.as_str() {
                "ALREADY_CONSUMED" => 409,
                "STORAGE_ERROR" => 503,
                "NOT_FOUND" | "CANDIDATE_NOT_FOUND" => 404,
                _ => 422,
            };
            make_json_resp(
                status_code,
                json!({
                    "status": "REJECTED",
                    "code": code,
                    "error": err
                }),
            )
        }
    }
}

pub fn handle_release_handoff(service: &DailyNewsService, body: &Value) -> Value {
    let cid = body["candidate_id"].as_str().unwrap_or("").trim();
    let token = body["token"].as_str().unwrap_or("").trim();
    if cid.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'candidate_id'" }),
        );
    }
    if token.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'token'" }),
        );
    }
    match service.release_handoff(cid, token) {
        Ok(true) => make_json_resp(200, json!({ "status": "RELEASED", "candidate_id": cid })),
        Ok(false) => make_json_resp(404, json!({ "status": "NOT_FOUND", "candidate_id": cid })),
        Err(e) => make_json_resp(422, json!({ "status": "REJECTED", "error": e })),
    }
}

pub fn handle_commit_handoff(service: &DailyNewsService, body: &Value, req: &Value) -> Value {
    let cid = body["candidate_id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .or_else(|| body["candidate"]["id"].as_str())
        .unwrap_or("")
        .trim();

    if cid.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'candidate_id'" }),
        );
    }

    let idempotency_key = body["idempotency_key"]
        .as_str()
        .or_else(|| body["idempotencyKey"].as_str())
        .or_else(|| req.get("headers").and_then(|h| h["idempotency-key"].as_str()))
        .or_else(|| req.get("headers").and_then(|h| h["x-idempotency-key"].as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let token = body["token"]
        .as_str()
        .or_else(|| body["reservation_token"].as_str())
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        return make_json_resp(
            400,
            json!({ "error": "Missing required parameter: 'token'" }),
        );
    }

    match service.commit_handoff_with_idempotency(cid, token, idempotency_key) {
        Ok(true) => make_json_resp(
            200,
            json!({
                "status": "PUBLISHED",
                "candidate_id": cid,
                "idempotent": idempotency_key.is_some()
            }),
        ),
        Ok(false) => make_json_resp(404, json!({ "status": "NOT_FOUND", "candidate_id": cid })),
        Err(e) => make_json_resp(422, json!({ "status": "REJECTED", "error": e })),
    }
}

pub fn handle_publish_idempotency(service: &DailyNewsService, method: &str, body: &Value) -> Value {
    if method == "GET" {
        let key = body["key"].as_str().unwrap_or("").trim();
        if key.is_empty() {
            return make_json_resp(400, json!({ "error": "Missing required query parameter: 'key'" }));
        }
        return match service.check_publish_idempotency(key) {
            Ok(Some(record)) => make_json_resp(
                200,
                json!({
                    "status": "RECORDED",
                    "exists": true,
                    "record": record
                }),
            ),
            Ok(None) => make_json_resp(
                200,
                json!({
                    "status": "AVAILABLE",
                    "exists": false
                }),
            ),
            Err(e) => make_json_resp(500, json!({ "error": e })),
        };
    }

    if method == "POST" {
        let key = body["key"].as_str().unwrap_or("").trim();
        let cid = body["candidate_id"].as_str().unwrap_or("").trim();
        let action = body["action"].as_str().unwrap_or("publish_upload").trim();
        if key.is_empty() || cid.is_empty() {
            return make_json_resp(
                400,
                json!({ "error": "Missing required fields: 'key' and 'candidate_id'" }),
            );
        }
        return match service.register_publish_idempotency(key, cid, action) {
            Ok(true) => make_json_resp(
                200,
                json!({
                    "status": "REGISTERED",
                    "registered": true,
                    "key": key,
                    "candidate_id": cid
                }),
            ),
            Ok(false) => {
                let existing = service.check_publish_idempotency(key).ok().flatten();
                make_json_resp(
                    409,
                    json!({
                        "status": "DUPLICATE",
                        "registered": false,
                        "key": key,
                        "existing": existing
                    }),
                )
            }
            Err(e) => make_json_resp(500, json!({ "error": e })),
        };
    }

    make_json_resp(405, json!({ "error": "Method Not Allowed" }))
}
