//! Declarative MCP Tool Schemas and Descriptions.
//!
//! Exposes available MCP tools in standard MCP JSON Schema format for connected AI agents.

use serde_json::{json, Value};

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required
    })
}

fn candidate_id_schema(description: &str) -> Value {
    object_schema(
        json!({
            "candidate_id": {
                "type": "string",
                "description": description
            }
        }),
        &["candidate_id"],
    )
}

fn candidate_token_schema(cand_desc: &str, token_desc: &str, extra_props: Option<serde_json::Map<String, Value>>) -> Value {
    let mut props = serde_json::Map::new();
    props.insert(
        "candidate_id".to_string(),
        json!({
            "type": "string",
            "description": cand_desc
        }),
    );
    props.insert(
        "token".to_string(),
        json!({
            "type": "string",
            "description": token_desc
        }),
    );
    if let Some(extras) = extra_props {
        for (k, v) in extras {
            props.insert(k, v);
        }
    }
    object_schema(Value::Object(props), &["candidate_id", "token"])
}

pub fn get_mcp_tools_list() -> Vec<Value> {
    let mut release_extras = serde_json::Map::new();
    release_extras.insert(
        "reason".to_string(),
        json!({
            "type": "string",
            "description": "Optional reason for releasing the reservation"
        }),
    );

    vec![
        json!({
            "name": "discover_candidates",
            "description": "Discover and score fresh tech and AI news candidates filtered by topic, date window, and quality thresholds.",
            "inputSchema": object_schema(
                json!({
                    "topics": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of topics (e.g. ['rust', 'llm', 'security'])"
                    },
                    "from_date": {
                        "type": "string",
                        "description": "ISO 8601 start date timestamp (e.g. '2026-11-01T00:00:00Z')"
                    },
                    "to_date": {
                        "type": "string",
                        "description": "ISO 8601 end date timestamp (e.g. '2026-11-04T00:00:00Z')"
                    },
                    "sources": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional feed sources (e.g. ['hackernews', 'reddit'])"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "default": 10,
                        "description": "Maximum number of candidate articles to return"
                    }
                }),
                &[],
            )
        }),
        json!({
            "name": "get_candidate",
            "description": "Retrieve full candidate details, research evidence fields, and publication metadata by candidate ID.",
            "inputSchema": candidate_id_schema("Unique canonical candidate identifier (e.g. 'cand_0123456789abcdef')")
        }),
        json!({
            "name": "enrich_candidate",
            "description": "Enrich a candidate article with captured primary source snapshot, structured claims, and visual evidence.",
            "inputSchema": object_schema(
                json!({
                    "candidate_id": {
                        "type": "string",
                        "description": "Candidate ID to enrich"
                    },
                    "override_url": {
                        "type": "string",
                        "description": "Optional explicit primary source URL if different from discovery canonical URL"
                    },
                    "claims": {
                        "type": "array",
                        "description": "List of grounded factual claims extracted from the article"
                    },
                    "visual_evidence": {
                        "type": "array",
                        "description": "List of validated visual assets (charts, diagrams, code blocks)"
                    }
                }),
                &["candidate_id"],
            )
        }),
        json!({
            "name": "verify_research_readiness",
            "description": "Score video production readiness (VIDEO_READY, NEEDS_RESEARCH, NEEDS_VERIFICATION, NEEDS_VISUAL_EVIDENCE) with strict evidence grounding.",
            "inputSchema": candidate_id_schema("Candidate ID to evaluate for video production readiness")
        }),
        json!({
            "name": "verify_production_handoff",
            "description": "Verify that a candidate article meets strict VIDEO_READY readiness criteria, check freshness, and perform atomic claim for production pipeline handoff.",
            "inputSchema": candidate_id_schema("Candidate ID to verify and claim for production handoff")
        }),
        json!({
            "name": "release_production_handoff",
            "description": "Release an active production handoff reservation using its secret reservation token, returning the candidate to the available pool.",
            "inputSchema": candidate_token_schema(
                "Unique candidate identifier whose reservation is to be released",
                "Secret reservation token received during handoff verification",
                Some(release_extras),
            )
        }),
        json!({
            "name": "commit_production_handoff",
            "description": "Commit an active production handoff reservation as PUBLISHED using its secret reservation token, permanently completing the handoff lifecycle.",
            "inputSchema": candidate_token_schema(
                "Unique candidate identifier to mark permanently PUBLISHED",
                "Secret reservation token received during handoff verification",
                None,
            )
        }),
        json!({
            "name": "get_evidence_snapshot",
            "description": "Fetch the immutable, SHA-256 content-addressed text snapshot of a captured primary or secondary source.",
            "inputSchema": object_schema(
                json!({
                    "sha256": {
                        "type": "string",
                        "pattern": "^[a-fA-F0-9]{64}$",
                        "description": "64-character hex SHA-256 hash of the snapshot artifact"
                    }
                }),
                &["sha256"],
            )
        }),
        json!({
            "name": "get_discussion",
            "description": "Retrieve community reactions, commentary trees, and sentiment for a news story (explicitly marked as community opinion, not factual support).",
            "inputSchema": object_schema(
                json!({
                    "url_or_id": {
                        "type": "string",
                        "description": "Discussion URL or post ID (e.g. Hacker News story ID or Reddit permalink)"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "default": 20,
                        "description": "Maximum number of comments to retrieve"
                    }
                }),
                &["url_or_id"],
            )
        }),
    ]
}
