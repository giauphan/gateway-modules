# Daily News & Content Discovery Service Module (`daily-news`)

A high-performance standalone provider module for **Toolcase Gateway**, extracted from OpenMontage's daily news gathering, canonicalization, and tiered ranking pipeline.

The module supports **dual-runtime execution**:
1. **Native Rust Binary (`daily-news-rs`)**: Standalone, high-speed, zero-dependency compiled binary for Rust-only or minimal container environments where Python 3 is not installed.
2. **Python Engine**: Dynamic, full-featured Python service with rich source adapters (NewsNow, Redis, Hacker News, GitHub, Product Hunt).

---

## Table of Contents

1. [Features & Capabilities](#1-features--capabilities)
2. [Dual-Runtime Architecture](#2-dual-runtime-architecture)
3. [Running in Pure Rust Environments (Zero Python)](#3-running-in-pure-rust-environments-zero-python)
4. [API Endpoints Reference](#4-api-endpoints-reference)
5. [Discovery API & Dual-Link Schema](#5-discovery-api--dual-link-schema)
6. [Pagination & Filtering Parameters](#6-pagination--filtering-parameters)
7. [Deterministic 6-Axis Scoring](#7-deterministic-6-axis-scoring)
8. [Configuration Options](#8-configuration-options)
9. [Integration with Toolcase Gateway](#9-integration-with-toolcase-gateway)

---

## 1. Features & Capabilities

- **Zero-Dependency Native Execution**: Runs directly as a compiled Rust binary communicating over standard `stdio-stream` JSON-RPC.
- **Multi-Source Ingestion**: Ingests breaking news and technical releases from Hacker News, GitHub, Product Hunt, NewsNow feeds, and Redis streams/lists.
- **SSRF Protection & URL Canonicalization**: Enforces strict URL sanitization, stripping tracking parameters (`utm_*`, `fbclid`, `gclid`, `ref`, `source`) and URL fragments.
- **Dual-Link Resolution**: Disambiguates between the actual content target (`canonical_url`) and the community discussion thread (`discussion_url`).
- **Deterministic Deduplication & Hashing**: Computes authoritative SHA-256 `title_hash` and `content_hash` to reject duplicates across multiple aggregators.
- **Explainable 6-Axis Scoring**: Computes objective scores across Freshness, Publisher Authority, Fact Density, Visual Potential, Audience Interest, and Feasibility.
- **Tiered Cascade Discovery**: Employs Tier 1 (Curated Queue) -> Tier 2 (Automated Breaking Feed) -> Tier 3 (Fail-Closed Safety).

---

## 2. Dual-Runtime Architecture

Toolcase Gateway executes modules via `stdio-stream` using the `./run.sh` launcher.

```text
Toolcase Gateway (Reverse Proxy / Routing)
   │
   ▼ stdio-stream (JSON Lines over stdin / stdout)
./run.sh
   ├── 1. Check $SCRIPT_DIR/rust/target/release/daily-news  ──► [Native Rust Engine]
   ├── 2. Check $SCRIPT_DIR/daily-news (Static Binary)     ──► [Native Rust Engine]
   └── 3. Fallback: command -v python3 adapter.py           ──► [Python Engine]
```

### Module File Layout

```text
modules/daily-news/
├── module.json         # Toolcase Gateway personal manifest (id: daily-news)
├── run.sh              # Universal auto-detecting executable launcher
├── adapter.py          # Python stdio-stream protocol adapter
├── config.example.json # Sample configuration template
├── README.md           # Comprehensive documentation
├── rust/               # Standalone Native Rust Crate
│   ├── Cargo.toml      # Minimal zero-async dependencies (serde, ureq, sha2, uuid)
│   └── src/
│       ├── domain.rs   # Domain entities, URL canonicalizer, SHA-256 hashing
│       ├── scoring.rs  # 6-axis scoring engine and candidate ranker
│       ├── lib.rs      # Library facade and unit test suite
│       └── main.rs     # stdio-stream event loop & route handlers
├── daily_news/         # Python domain & application package
│   ├── domain.py       # Domain models, DiscoveryResult, RawNewsItem
│   ├── scoring.py      # Python 6-axis scoring implementation
│   ├── discovery.py    # Tiered cascade discovery engine
│   ├── fetcher.py      # NewsNow, Hacker News, Redis fetchers
│   └── ui.py           # Interactive HTML dashboard renderer
└── tests/              # Python test suite
```

---

## 3. Running in Pure Rust Environments (Zero Python)

If your client, container, or server environment does **not** have Python installed, you can build the native Rust binary with `cargo`:

### Step 1: Build the Native Binary

```bash
cd modules/daily-news/rust
cargo build --release
```

The resulting binary will be created at `rust/target/release/daily-news`.

*(Optional)* You can copy the binary directly to the module root:
```bash
cp target/release/daily-news ../daily-news
```

### Step 2: Verify Execution

Run `./run.sh` or check the module via Toolcase Gateway:
```bash
curl -s http://127.0.0.1:28130/custom/daily-news/health
```

Output:
```json
{
  "status": "healthy",
  "module": "daily-news",
  "version": "1.3.0",
  "engine": "rust-native"
}
```

The `x-module-engine: rust-native` response header confirms that the native Rust engine is actively handling requests without invoking Python.

---

## 4. API Endpoints Reference

All endpoints are mounted under the Toolcase Gateway prefix `/custom/daily-news/*`:

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/custom/daily-news/health` | Health check, module version, and active runtime engine |
| `GET` | `/custom/daily-news/models` | OpenAI-compatible model listing (`daily-news`) |
| `POST` | `/custom/daily-news/chat/completions` | OpenAI-compatible chat completion returning formatted news digest |
| `GET` | `/custom/daily-news/ui` or `/` | Interactive responsive HTML status dashboard |
| `POST` | `/custom/daily-news/api/v1/discover` | Tiered content discovery returning ranked candidates and top-level `count` |
| `GET` | `/custom/daily-news/api/v1/candidates` | Returns currently ranked and scored candidates |
| `GET` | `/custom/daily-news/api/v1/feed` | Ingested raw feed items before deduplication and scoring |
| `POST` | `/custom/daily-news/api/v1/consume` | Marks candidates as consumed/processed to avoid re-serving |
| `GET` | `/custom/daily-news/api/v1/config` | Inspect active module configuration |
| `POST` | `/custom/daily-news/api/v1/config` | Update configuration parameters dynamically |

---

## 5. Discovery API & Dual-Link Schema

### Request Format (`POST /custom/daily-news/api/v1/discover`)

The discovery endpoint accepts parameters either via JSON body or query parameters:

```bash
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/discover \
  -H "Content-Type: application/json" \
  -d '{
    "limit": 15,
    "from_date": "2026-11-01T00:00:00Z",
    "to_date": "2026-11-02T23:59:59Z",
    "topics": ["agent_architecture", "agent_tool_use"]
  }'
```

Query string format is also fully supported:
```bash
curl -s -X POST "http://127.0.0.1:28130/custom/daily-news/api/v1/discover?limit=15"
```

### Response Schema

The response includes the **top-level `count`** field representing the exact number of returned candidates, along with the discovery tier and full candidate metadata:

```json
{
  "tier": "tier_2_automated",
  "status": "ELIGIBLE",
  "count": 15,
  "candidates": [
    {
      "candidate_id": "cand_2958a03a073a1d37",
      "canonical_url": "https://syscall.sh",
      "original_url": "https://syscall.sh/",
      "discussion_url": "https://news.ycombinator.com/item?id=50334704",
      "title": "Show HN: I've updated my Linux System Call website and API",
      "publisher": "syscall.sh",
      "normalized_publisher": "syscall.sh",
      "published_at_utc": "2026-11-02T09:29:16+00:00",
      "content_hash": "2958a03a073a1d37b332b6c484093e0932f39e888cf67e7424fd1324e4cbe8ec",
      "title_hash": "ee2ab31b17be9bd53fe8f7c28477bf473c37ded461c141e44eb8f66a886d270d",
      "topic_family": "agent_architecture",
      "tags": ["hackernews", "tech"],
      "scoring": {
        "total_score": 0.765,
        "axes": {
          "freshness": 1.0,
          "publisher_authority": 0.5,
          "fact_density": 0.7,
          "visual_potential": 0.5,
          "audience_interest": 1.0,
          "feasibility": 1.0
        }
      }
    }
  ],
  "source_file": "live:hackernews",
  "diagnostics": {
    "tier_2_checked": true
  }
}
```

### Understanding the Dual Links

Every candidate provides both links explicitly:
1. **`canonical_url`**: The destination article, project website, or announcement. Tracking parameters (`utm_source`, `utm_medium`, `fbclid`, `gclid`, `ref`) and fragments are stripped for clean caching and deduplication.
2. **`discussion_url`**: The community comment thread or forum post (e.g. `https://news.ycombinator.com/item?id=50334704`). This allows consumers to read community reactions, upvotes, and context.

---

## 6. Pagination & Filtering Parameters

| Parameter | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `limit` (or `count`) | Integer | `5` | Maximum number of candidate items to return (1 to 100). |
| `from_date` | ISO 8601 String | `None` | Filters candidates published on or after this timestamp. |
| `to_date` | ISO 8601 String | `None` | Filters candidates published on or before this timestamp. |
| `topics` | List[String] | `None` | Filters by topic family (e.g., `agent_architecture`, `context_memory`, `agent_tool_use`, `debugging_reliability`). |
| `source_feeds` | List[String] | `None` | Limits discovery to specific feed identifiers (e.g., `["live:hackernews"]`). |

---

## 7. Deterministic 6-Axis Scoring

Candidates are ranked using a multi-factor formula:

$$\text{Score} = (W_F \times F) + (W_A \times A) + (W_D \times D) + (W_V \times V) + (W_I \times I) + (W_{\text{Fe}} \times \text{Fe})$$

| Axis | Weight | Meaning | Scoring Criteria |
| :--- | :--- | :--- | :--- |
| **Freshness ($F$)** | 25% | Timeliness of publication | $1.0$ for $<12\text{h}$, decays linearly to $0.0$ at $48\text{h}$. |
| **Publisher Authority ($A$)** | 20% | Domain reputation | $1.0$ for tier-1 AI labs / peer-reviewed sources; $0.70$ for HN/TechCrunch; $0.50$ default. |
| **Fact Density ($D$)** | 15% | Substance of content | Evaluated on text length, technical terminology, and concrete metrics. |
| **Visual Potential ($V$)** | 15% | Media suitability | $0.8$ if rich media/diagrams present; $0.6$ if active discussion exists. |
| **Audience Interest ($I$)** | 15% | Community engagement | Upvotes, comment count, and social velocity. |
| **Feasibility ($\text{Fe}$)** | 10% | Reproducibility | $1.0$ if code, open API, or runnable repo is linked. |

---

## 8. Configuration Options

Module settings can be specified in `config.json` inside the module directory:

```json
{
  "newsnow_url": "http://127.0.0.1:4444",
  "freshness_window_hours": 48.0,
  "max_age_days": 7,
  "require_latest_news": true,
  "default_sources": ["hackernews", "github", "producthunt"],
  "redis_enabled": false,
  "redis_url": "redis://localhost:6379/0",
  "redis_key": "news:incoming",
  "redis_type": "stream"
}
```

---

## 9. Integration with Toolcase Gateway

### OpenAI-Compatible Chat Integration

Query the module as an OpenAI-compatible model:

```bash
curl -s http://127.0.0.1:28130/custom/daily-news/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "daily-news",
    "messages": [{"role": "user", "content": "What is breaking in tech today?"}],
    "limit": 10
  }' | jq '.choices[0].message.content'
```

### Module Auto-Discovery in Toolcase Gateway

When installed in `./addons/daily-news/`, Toolcase Gateway automatically discovers, registers, and mounts the module routes on server startup or dynamically upon incoming requests.
