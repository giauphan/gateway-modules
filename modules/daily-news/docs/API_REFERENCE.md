# Daily News Module — Complete API & Protocol Reference

This document provides exhaustive documentation for all HTTP endpoints, JSON-RPC stdio-stream envelopes, request parameters, response schemas, and error codes supported by `daily-news` (v1.19.4).

---

## 1. Protocol Architecture & Transports

The `daily-news` module operates via two interchangeable transport layers:
1. **Toolcase Gateway Host Mounting**: Mounted under the namespace `/custom/daily-news/*` on Toolcase Gateway (default: `http://127.0.0.1:28130/custom/daily-news`).
2. **Standard I/O Stream (stdio-stream)**: Standard Unix lines of newline-delimited JSON envelopes exchanged across `stdin` and `stdout`.

### Stdio-Stream Request Envelope
```json
{
  "path": "/api/v1/discover?limit=5",
  "method": "POST",
  "headers": {
    "content-type": "application/json"
  },
  "body": "{\"topics\": [\"agent_tool_use\"], \"limit\": 5}"
}
```

### Stdio-Stream Response Envelope
```json
{
  "status": 200,
  "headers": {
    "content-type": "application/json",
    "x-module-engine": "rust-native"
  },
  "body": "{\"tier\":\"tier_2_automated\",\"status\":\"ELIGIBLE\",\"count\":1,\"candidates\":[...]}"
}
```

---

## 2. Common Query & Body Parameters

Every endpoint that accepts options supports unified parameter resolution: parameters can be passed as URL query strings or JSON object keys in the request body.

| Parameter | Aliases | Type | Default | Description |
| :--- | :--- | :--- | :--- | :--- |
| `limit` | `count` | Integer | `5` | Maximum number of candidates or feed items to return. |
| `from_date` | `since`, `start_date`, `from` | String | `None` | Lower bound date (`YYYY-MM-DD` or ISO 8601 UTC). Inclusive start of day (`00:00:00Z`). |
| `to_date` | `until`, `end_date`, `to` | String | `None` | Upper bound date (`YYYY-MM-DD` or ISO 8601 UTC). Inclusive end of day (`23:59:59Z`). |
| `topics` | `topic` | Array or CSV String | `None` | Topic filter (e.g. `["agent_tool_use"]` or `"agent_tool_use,context_memory"`). |
| `sources` | `source` | Array or CSV String | Configured | Specific feed sources to query (e.g. `hackernews`, `reddit`, `redis`, `github`). |
| `curated_candidates`| `curated`, `items` | Array of Objects | `None` | Injected Tier-1 candidates for prioritized verification. |

---

## 3. Endpoints Catalog

### 3.1. Health Check
Returns the runtime health, engine type, version, and active counts.

- **Route**: `GET /health` (Gateway: `GET /custom/daily-news/health`)
- **Headers**: None required.

#### Example Request
```bash
curl -s http://127.0.0.1:28130/custom/daily-news/health
```

#### Example Response (200 OK)
```json
{
  "status": "healthy",
  "module": "daily-news",
  "version": "1.19.4",
  "engine": "rust-native",
  "consumed_count": 0,
  "cached_candidates": 5,
  "newsnow_url": "http://127.0.0.1:4444",
  "redis_enabled": false,
  "reddit_enabled": true,
  "reddit_subreddits": ["LocalLLaMA", "MachineLearning", "artificial", "technology"]
}
```

---

### 3.2. Models Discovery
OpenAI-compatible models catalog identifying the module's model ID.

- **Route**: `GET /models` (Gateway: `GET /custom/daily-news/models`)

#### Example Response (200 OK)
```json
{
  "object": "list",
  "data": [
    {
      "id": "daily-news",
      "object": "model",
      "created": 1793618000,
      "owned_by": "daily-news-rs"
    }
  ]
}
```

---

### 3.3. Interactive HTML Dashboard
Renders a responsive, dark-mode administrative dashboard summarizing candidate state, configuration, and recent feeds.

- **Route**: `GET /ui` or `GET /` (Gateway: `GET /custom/daily-news/ui` or `GET /custom/daily-news`)
- **Content-Type**: `text/html; charset=utf-8`

#### Example Request
```bash
curl -s http://127.0.0.1:28130/custom/daily-news/ui
```

---

### 3.4. Interactive Architecture Flowchart
Renders an interactive, standalone HTML5 + SVG architecture flowchart with 6 visual exploration tabs (Clean Architecture layers, multi-feed pipeline, cascading discovery, SSRF source capture, research readiness state machine, and 6-axis scoring).

- **Route**: `GET /flow` or `GET /flowchart` (Gateway: `GET /custom/daily-news/flow` or `GET /custom/daily-news/flowchart`)
- **Content-Type**: `text/html; charset=utf-8`

#### Example Request
```bash
curl -s http://127.0.0.1:28130/custom/daily-news/flow
```

---

### 3.5. Chat Completions
OpenAI-compatible completion endpoint rendering ranked daily stories in conversational markdown.

- **Route**: `POST /chat/completions` (Gateway: `POST /custom/daily-news/chat/completions`)

#### Example Request
```bash
curl -s http://127.0.0.1:28130/custom/daily-news/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "daily-news",
    "messages": [{"role": "user", "content": "What is the latest AI news today?"}],
    "limit": 3
  }'
```

#### Example Response (200 OK)
```json
{
  "id": "chatcmpl-a1b2c3d4",
  "object": "chat.completion",
  "model": "daily-news",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "### Daily News Briefing (3 items):\n\n1. **Dynamic MCP Tool Scoping** (Anthropic) [Score: 0.88]\n   https://docs.anthropic.com/agent-tools\n   Discussion: https://news.ycombinator.com/item?id=50334704"
      },
      "finish_reason": "stop"
    }
  ]
}
```

---

### 3.5. Tiered Content Discovery
Executes the three-tier cascade discovery algorithm, returning ranked candidates with complete 6-beat engineering evidence fields.

- **Route**: `POST /api/v1/discover` or `GET /api/v1/discover`
- **Gateway**: `/custom/daily-news/api/v1/discover`

#### Example Request (With Date Filter & Topic Scope)
```bash
curl -s "http://127.0.0.1:28130/custom/daily-news/api/v1/discover?from_date=2026-11-01&to_date=2026-11-02&topics=agent_tool_use&limit=2"
```

#### Example Response (200 OK)
```json
{
  "tier": "tier_2_automated",
  "status": "ELIGIBLE",
  "count": 1,
  "source_file": "live_feeds",
  "reason": null,
  "diagnostics": {
    "tier": "tier_2_automated"
  },
  "candidates": [
    {
      "candidate_id": "cand_eb6f9d5de3c4d7e1",
      "canonical_url": "https://docs.anthropic.com/agent-tools",
      "original_url": "https://docs.anthropic.com/agent-tools?utm_source=twitter",
      "url": "https://docs.anthropic.com/agent-tools",
      "source_url": "https://docs.anthropic.com/agent-tools",
      "source_type": "news",
      "title": "Model Context Protocol Tool Schema Update",
      "publisher": "Anthropic",
      "normalized_publisher": "Anthropic",
      "published_at": "2026-11-02T10:00:00+00:00",
      "published_at_utc": "2026-11-02T10:00:00+00:00",
      "content_hash": "eb6f9d5de3c4d7e1aaee2319bec5793652f54df279e24404de26645714f25e66",
      "title_hash": "9f85d9cb0403270bd3c7882b886468ed2e860fe172d6ced2ee436155107617db",
      "topic_family": "agent_tool_use",
      "discussion_url": "https://news.ycombinator.com/item?id=50334704",
      "adapter_version": "1.3.0",
      "author": "Engineering Team",
      "description": "Dynamic tool scoping specifications for multi-server MCP environments.",
      "body": null,
      "summary": "Dynamic tool scoping specifications for multi-server MCP environments.",
      "tags": ["anthropic", "mcp", "tools"],
      "language": "en",
      "image_url": null,
      "is_evergreen": false,
      "content_kind": "news",
      "updated_at": null,
      "hook": "Is your coding agent choking on 50 different MCP tool definitions?",
      "viewer_question": "How do you scope tool schemas dynamically?",
      "symptom": "Agent context window overwhelmed by thousands of lines of unused tool schemas.",
      "root_cause": "Eager registration dumps all schemas into initial prompt.",
      "remedy": "Implement two-tier tool resolution and lazy schema loading.",
      "evidence_excerpt": "Tool schemas may now be dynamically requested via search.",
      "source_snapshot": "artifacts/mcp_spec.txt",
      "snapshot_sha256": "2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72",
      "independent_fact_check": null,
      "raw_provenance": {
        "source_feed": "daily-news",
        "source_id": "hn_50334704",
        "extra": {
          "by": "author",
          "score": 142
        }
      },
      "scoring": {
        "total_score": 0.8834,
        "axes": {
          "freshness": 1.0,
          "publisher_authority": 1.0,
          "fact_density": 0.8,
          "visual_potential": 0.7,
          "audience_interest": 0.9,
          "feasibility": 1.0
        }
      }
    }
  ]
}
```

---

### 3.6. Candidates Normalization & Scoring
Accepts raw news items and transforms them into normalized, deduplicated, and scored candidates.

- **Route**: `GET /api/v1/candidates` or `POST /api/v1/candidates`
- **Gateway**: `/custom/daily-news/api/v1/candidates`

#### Example Request
```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/candidates \
  -H "Content-Type: application/json" \
  -d '{
    "items": [
      {
        "id": "raw_1",
        "url": "https://example.com/ai-update?utm_source=feed",
        "title": "Autonomous Coding Agent Benchmark Matrix",
        "published_at": "2026-11-02T08:00:00+00:00",
        "symptom": "Flaky retries in CI pipelines",
        "root_cause": "Unstable nondeterministic agent seed",
        "remedy": "Pin evaluation seeds and cache fixtures"
      }
    ],
    "limit": 5
  }'
```

---

### 3.7. Raw Feed Ingestion
Returns raw news items from configured feeds prior to normalization or scoring.

- **Route**: `GET /api/v1/feed` or `POST /api/v1/feed`
- **Gateway**: `/custom/daily-news/api/v1/feed?limit=10`

---

### 3.8. Discussion Thread Inspection
Fetches an in-depth, structured discussion thread (from Reddit or Hacker News) including author, score, post selftext, and recursive comment trees.

- **Route**: `GET /api/v1/discussion` or `POST /api/v1/discussion`
- **Gateway**: `/custom/daily-news/api/v1/discussion`

#### Request Parameters
- `url` or `discussion_url` or `id`: Hacker News item ID (e.g. `50346981`, `hn_50346981`, or full HN URL) or Reddit URL / Post ID (e.g. `1xmitzx`, `reddit_LocalLLaMA_1xmitzx`, `https://reddit.com/r/LocalLLaMA/comments/1xmitzx/...`).
- `limit`: (Optional) Maximum number of top-level comments to retrieve (default: 20 for HN, 50 for Reddit).

#### Example Request
```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/discussion \
  -H "Content-Type: application/json" \
  -d '{"url": "https://news.ycombinator.com/item?id=50346981", "limit": 5}'
```

#### Example Response (200 OK)
```json
{
  "id": "50346981",
  "subreddit": "hackernews",
  "title": "Show HN: Hacker Times",
  "author": "nytimes_dev",
  "permalink": "https://news.ycombinator.com/item?id=50346981",
  "url": "https://hackertimes.com",
  "selftext": null,
  "score": 142,
  "num_comments": 38,
  "comments": [
    {
      "id": "50347100",
      "author": "alice",
      "body": "Love the serif typography and layout density!",
      "score": 15,
      "created_utc": 1730545800,
      "created_at": "2026-11-02T11:10:00+00:00",
      "permalink": "https://news.ycombinator.com/item?id=50347100",
      "replies": []
    }
  ]
}
```

---

### 3.9. Mark Consumed
Marks candidate IDs as consumed in the persistent store (`.state/consumed.json`). Consumed candidates are never re-served by the discovery cascade.

- **Route**: `POST /api/v1/consume`
- **Gateway**: `/custom/daily-news/api/v1/consume`

#### Example Request
```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/consume \
  -H "Content-Type: application/json" \
  -d '{"candidate_ids": ["cand_eb6f9d5de3c4d7e1"]}'
```

#### Example Response (200 OK)
```json
{
  "status": "consumed",
  "count": 1
}
```

---

### 3.10. Configuration Management
Inspect or update runtime settings dynamically without restarting the gateway process.

- **Route**: `GET /api/v1/config` (Read) or `POST /api/v1/config` (Update)
- **Gateway**: `/custom/daily-news/api/v1/config`

#### Example Request (Update Settings)
```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/config \
  -H "Content-Type: application/json" \
  -d '{
    "redis_enabled": true,
    "redis_url": "redis://127.0.0.1:6379/0",
    "redis_key": "news:incoming",
    "redis_type": "stream",
    "reddit_enabled": true,
    "reddit_subreddits": ["LocalLLaMA", "MachineLearning", "artificial"]
  }'
```

#### Example Response (200 OK)
```json
{
  "status": "updated",
  "config": {
    "newsnow_url": "http://127.0.0.1:4444",
    "freshness_window_hours": 48.0,
    "max_age_days": 7,
    "require_latest_news": true,
    "default_sources": ["hackernews", "reddit", "github", "producthunt"],
    "redis_enabled": true,
    "redis_url": "redis://127.0.0.1:6379/0",
    "redis_key": "news:incoming",
    "redis_type": "stream",
    "reddit_enabled": true,
    "reddit_subreddits": ["LocalLLaMA", "MachineLearning", "artificial"],
    "reddit_client_id": null,
    "reddit_client_secret": null,
    "reddit_base_url": null
  }
}
```

### 3.11. Primary Source Capture & Enrichment
Safely fetches external primary source articles, enforces strict SSRF checks at each redirect hop, computes cryptographic SHA-256 signatures of actual saved bytes, and stores content-addressed snapshots in `.state/evidence/`.

- **Route**: `POST /api/v1/enrich` or `POST /api/v1/capture`
- **Gateway**: `/custom/daily-news/api/v1/enrich`

#### Example Request
```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/enrich \
  -H "Content-Type: application/json" \
  -d '{"url": "https://honnibal.dev/blog/post"}'
```

#### Example Response (200 OK)
```json
{
  "url": "https://honnibal.dev/blog/post",
  "fetch_status": "SUCCESS",
  "content_type": "text/html; charset=utf-8",
  "title": "Architectural Scaling with Rust & LLMs",
  "published_at": null,
  "excerpt": "We demonstrated a 4x reduction in cache latency using deterministic memory management...",
  "snapshot_ref": ".state/evidence/2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72.txt",
  "snapshot_sha256": "2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72",
  "captured_at": "2026-11-03T14:30:00Z",
  "extraction_method": "conservative_html"
}
```

---

### 3.12. Candidate Persistent Enrichment Workflow
Enriches an existing candidate by `candidate_id`, captures its primary source, evaluates content quality gates against anti-bot interstitials, grounds claims, validates visual evidence, updates the candidate's research readiness (e.g. to `VIDEO_READY`), and updates the persistent cache.

- **Route**: `POST /api/v1/enrich-candidate` or `POST /api/v1/enrich` (with `candidate_id`)
- **Gateway**: `/custom/daily-news/api/v1/enrich-candidate`

#### Example Request
```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/enrich-candidate \
  -H "Content-Type: application/json" \
  -d '{
    "candidate_id": "cand_2bad0815c06b65db",
    "override_url": "https://example.com/article",
    "claims": [
      {
        "claim_id": "c1",
        "statement": "4x reduction in cache latency",
        "evidence_refs": [{
          "source_ref": "2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72",
          "excerpt": "4x reduction in cache latency using deterministic memory management.",
          "locator": "paragraph:1"
        }],
        "verification_status": "INDEPENDENTLY_VERIFIED",
        "reviewer": "Benchmark Lab QA"
      }
    ],
    "visual_evidence": [
      {
        "asset_ref": "vis_1",
        "kind": "chart",
        "source_url": "https://example.com/chart.png",
        "license_status": "cc-by",
        "provenance_status": "verified"
      }
    ]
  }'
```

#### Example Response (200 OK)
```json
{
  "candidate": {
    "candidate_id": "cand_2bad0815c06b65db",
    "title": "Architectural Scaling with Rust & LLMs",
    "research_readiness": {
      "status": "VIDEO_READY",
      "reasons": ["All primary source, claim, and visual verification criteria satisfied"],
      "missing_fields": [],
      "contract_version": "2.0"
    }
  },
  "production_scoring": {
    "verified_claim_count": 1,
    "source_quality": 1.0,
    "independent_confirmation_count": 1,
    "visual_asset_readiness": 1.0,
    "production_readiness_score": 1.0,
    "is_video_production_ready": true
  },
  "transition": "NEEDS_SOURCE -> VIDEO_READY"
}
```

---

### 3.13. Evidence Snapshot Retrieval
Retrieves the saved raw text/snapshot content for a given verified SHA-256 hash.

- **Route**: `GET /api/v1/evidence` or `POST /api/v1/evidence`
- **Gateway**: `/custom/daily-news/api/v1/evidence?sha256=2bad0815c06b...`

#### Example Response (200 OK)
```json
{
  "sha256": "2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72",
  "ref": ".state/evidence/2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72.txt",
  "content": "<!DOCTYPE html><html>...</html>"
}
```

---

## 4. Error Code Reference

When a request cannot be fulfilled, the service returns standardized JSON error responses with deterministic status codes:

| Error Code | HTTP Status | Trigger Condition | Example Response |
| :--- | :--- | :--- | :--- |
| `INVALID_DATE_RANGE` | `400 Bad Request` | `from_date` is later than `to_date`. | `{"tier": "none", "status": "INVALID_DATE_RANGE", "candidates": [], "reason": "from_date (\"2026-11-05\") must not be later than to_date (\"2026-11-01\")"}` |
| `INVALID_DATE_FORMAT` | `400 Bad Request` | Date string cannot be parsed as `YYYY-MM-DD` or ISO 8601. | `{"tier": "none", "status": "INVALID_DATE_FORMAT", "candidates": [], "reason": "from_date error: Invalid date part: bad-date"}` |
| `NO_ELIGIBLE_TOPIC` | `200 OK (count: 0)` | Fails closed when no candidates match topic or date scope. | `{"tier": "none", "status": "NO_ELIGIBLE_TOPIC", "count": 0, "candidates": [], "reason": "No eligible candidates matched the requested topic filter(s)..."}` |
| `Route not found` | `404 Not Found` | Unrecognized subpath requested. | `{"error": "Route not found: /unknown"}` |
| `Missing required parameter` | `400 Bad Request` | Discussion inspection missing required target URL or ID. | `{"error": "Missing required parameter: 'url', 'discussion_url', or 'id'"}` |
