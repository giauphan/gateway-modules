//! Declarative MCP Tool Schemas and Descriptions.
//!
//! Exposes available MCP tools in standard MCP JSON Schema format for connected AI agents.

use serde_json::{json, Value};

pub fn get_mcp_tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "discover_candidates",
            "description": "Discover and score fresh tech and AI news candidates filtered by topic, date window, and quality thresholds.",
            "inputSchema": {
                "type": "object",
                "properties": {
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
                }
            }
        }),
        json!({
            "name": "get_candidate",
            "description": "Retrieve full candidate details, research evidence fields, and publication metadata by candidate ID.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "candidate_id": {
                        "type": "string",
                        "description": "Unique canonical candidate identifier (e.g. 'cand_0123456789abcdef')"
                    }
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "enrich_candidate",
            "description": "Enrich a candidate article with captured primary source snapshot, structured claims, and visual evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
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
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "verify_research_readiness",
            "description": "Score video production readiness (VIDEO_READY, NEEDS_RESEARCH, NEEDS_VERIFICATION, NEEDS_VISUAL_EVIDENCE) with strict evidence grounding.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "candidate_id": {
                        "type": "string",
                        "description": "Candidate ID to evaluate for video production readiness"
                    }
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "verify_production_handoff",
            "description": "Verify that a candidate article meets strict VIDEO_READY readiness criteria, check freshness, and perform atomic claim for production pipeline handoff.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "candidate_id": {
                        "type": "string",
                        "description": "Candidate ID to verify and claim for production handoff"
                    }
                },
                "required": ["candidate_id"]
            }
        }),
        json!({
            "name": "get_evidence_snapshot",
            "description": "Fetch the immutable, SHA-256 content-addressed text snapshot of a captured primary or secondary source.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sha256": {
                        "type": "string",
                        "pattern": "^[a-fA-F0-9]{64}$",
                        "description": "64-character hex SHA-256 hash of the snapshot artifact"
                    }
                },
                "required": ["sha256"]
            }
        }),
        json!({
            "name": "get_discussion",
            "description": "Retrieve community reactions, commentary trees, and sentiment for a news story (explicitly marked as community opinion, not factual support).",
            "inputSchema": {
                "type": "object",
                "properties": {
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
                },
                "required": ["url_or_id"]
            }
        }),
    ]
}
