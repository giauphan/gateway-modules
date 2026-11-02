# Daily News & Content Discovery Service Module

A standalone provider module for **Toolcase Gateway**, extracted from OpenMontage's daily news gathering, canonicalization, and tiered ranking pipeline.

---

## 1. Capabilities & Features

- **Multi-Source News Ingestion**: Automatically pulls raw news items from NewsNow, Hacker News, GitHub, and Product Hunt.
- **SSRF Protection & URL Canonicalization**: Enforces strict URL sanitization, domain allowlisting, and tracking parameter stripping (`utm_*`, `fbclid`, `gclid`).
- **Deterministic Deduplication & Hashing**: Computes authoritative SHA-256 `title_hash` and `content_hash` signatures.
- **Freshness Window Enforcement**: Rejects stale feeds and articles beyond the configurable window (default 48h).
- **Multi-Axis Explainable Scoring**: Ranks content along six axes (freshness, publisher authority, fact density, visual potential, audience interest, feasibility).
- **Tiered Content Discovery**: Cascades through Tier 1 (Curated queue) -> Tier 2 (Automated feed) -> Tier 3 (Fail-Closed safety).
- **Standard Toolcase Gateway Protocol**: Speaks `stdio-stream` JSON-RPC over stdin/stdout with complete prefix isolation under `/custom/daily-news/*`.

---

## 2. API Endpoints

| Method | Path | Description |
| :--- | :--- | :--- |
| `GET` | `/custom/daily-news/` or `/ui` | Interactive responsive HTML status dashboard |
| `GET` | `/custom/daily-news/health` | Health check and module operational statistics |
| `GET` | `/custom/daily-news/models` | OpenAI-compatible models discovery |
| `POST`| `/custom/daily-news/chat/completions` | Formatted daily news summary in chat format |
| `GET` | `/custom/daily-news/api/v1/feed` | Raw ingested feed items |
| `GET` | `/custom/daily-news/api/v1/candidates`| Normalized, scored, and ranked candidates |
| `POST`| `/custom/daily-news/api/v1/discover` | Priority tiered discovery with topic and date-range filtering |
| `POST`| `/custom/daily-news/api/v1/consume` | Mark candidates as consumed / processed |
| `GET` | `/custom/daily-news/api/v1/config` | Read active configuration |
| `POST`| `/custom/daily-news/api/v1/config` | Update active configuration |

---

## 3. Configuration Reference

| Key | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `newsnow_url` | String | `http://127.0.0.1:4444` | Upstream NewsNow feed service base URL |
| `freshness_window_hours` | Number | `48.0` | Maximum age of news articles in hours to consider fresh |
| `max_age_days` | Number | `7` | Maximum age in days for tiered content discovery |
| `require_latest_news` | Boolean | `true` | Prioritize latest breaking news with verified publication timestamps |
| `default_sources` | List[String] | `["hackernews", "github", "producthunt"]` | Upstream feed IDs to query |
| `redis_enabled` | Boolean | `false` | Enable ingesting articles from Redis Stream or List |
| `redis_url` | String | `redis://localhost:6379/0` | Redis server connection URL |
| `redis_key` | String | `news:incoming` | Redis Stream or List key to read |
| `redis_type` | String | `stream` | Redis structure type: `stream` (XREVRANGE) or `list` (LRANGE) |

---

## 4. Architecture & Clean Layering

```text
daily-news/
├── module.json         # Toolcase Gateway manifest
├── run.sh              # Entrypoint launcher
├── adapter.py          # stdio-stream protocol adapter
├── config.example.json # Configuration template
├── daily_news/         # Domain & application service package
│   ├── domain.py       # Core entities, value objects, URL canonicalizer
│   ├── normalizer.py   # Schema normalization and batch deduplication
│   ├── fetcher.py      # Upstream NewsNow and file feed fetcher
│   ├── scoring.py      # Six-axis deterministic scoring & ranker
│   ├── discovery.py    # Tiered cascading content discovery
│   ├── storage.py      # Persistent cache and consumed tracker
│   └── ui.py           # Dashboard HTML renderer
└── tests/              # Unit and integration test suite
```
