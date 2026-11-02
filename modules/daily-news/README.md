# Daily News & Content Discovery Service Module (`daily-news`)

A high-performance standalone provider module for **Toolcase Gateway**, extracted from OpenMontage's daily news gathering, canonicalization, and tiered ranking pipeline.

The module is **100% Native Rust** with **zero Python dependencies**.

---

## Table of Contents

1. [Features & Capabilities](#1-features--capabilities)
2. [Clean Architecture & File Layout](#2-clean-architecture--file-layout)
3. [Running and Compiling with Cargo](#3-running-and-compiling-with-cargo)
4. [API Endpoints Reference](#4-api-endpoints-reference)
5. [Query String Parameters & Dual-Link Schema](#5-query-string-parameters--dual-link-schema)
6. [Deterministic 6-Axis Scoring](#6-deterministic-6-axis-scoring)
7. [Configuration Reference](#7-configuration-reference)

---

## 1. Features & Capabilities

- **100% Pure Native Rust**: Zero Python runtime, pip dependencies, or virtualenvs required.
- **Query String Parameter Support**: Accepts parameters seamlessly via JSON body or URL query strings (`?limit=15`, `?count=10`, `?topics=agent_architecture`) across all routes.
- **Multi-Source News Ingestion**: Ingests breaking stories from Hacker News Firebase API and upstream NewsNow instances.
- **SSRF Protection & URL Canonicalization**: Enforces strict URL sanitization, stripping tracking parameters (`utm_*`, `fbclid`, `gclid`, `ref`, `source`) and URL fragments.
- **Dual-Link Resolution**: Disambiguates between the actual content target (`canonical_url`) and the community discussion thread (`discussion_url`).
- **Deterministic Deduplication & Hashing**: Computes authoritative SHA-256 `title_hash` and `content_hash` signatures.
- **Explainable 6-Axis Scoring**: Computes objective scores across Freshness, Publisher Authority, Fact Density, Visual Potential, Audience Interest, and Feasibility.
- **Tiered Cascade Discovery**: Employs Tier 1 (Curated Queue) -> Tier 2 (Automated Breaking Feed) -> Tier 3 (Fail-Closed Safety).
- **Interactive Dark-Mode UI**: Built-in responsive HTML status dashboard mounted at `/ui`.

---

## 2. Clean Architecture & File Layout

The module is organized strictly according to Clean Architecture and Domain-Driven Design (DDD). Every file remains under 450 lines:

```text
modules/daily-news/
├── Cargo.toml          # Rust crate manifest (serde, ureq, sha2, uuid)
├── Cargo.lock          # Locked deterministic dependencies
├── daily-news          # Precompiled standalone ELF binary (2.4 MB stripped)
├── run.sh              # Standalone executable launcher
├── module.json         # Toolcase Gateway manifest
├── config.example.json # Configuration template
├── README.md           # Comprehensive documentation
└── src/
    ├── domain.rs       # Domain Entities, Value Objects, URL canonicalizer, SHA-256
    ├── ports.rs        # Outbound Port Traits (FeedFetcherPort, StoragePort)
    ├── scoring.rs      # Domain Service: Deterministic 6-axis scoring & ranker
    ├── service.rs      # Application Layer: Discovery, Consume, and Config use cases
    ├── storage.rs      # Storage Adapter: Persistent JSON state & candidate cache
    ├── fetcher.rs      # Network Adapter: Live Hacker News & NewsNow HTTP client
    ├── ui.rs           # Presenter Adapter: Responsive HTML dashboard presenter
    ├── lib.rs          # Library facade and module re-exports
    └── main.rs         # Frameworks & Drivers: stdio-stream JSON-RPC event loop
```

---

## 3. Running and Compiling with Cargo

### Precompiled Standalone Mode
The module includes a precompiled, stripped 2.4 MB binary (`./daily-news`). When Toolcase Gateway starts, `./run.sh` immediately executes this binary:
```bash
./run.sh
```

### Compiling from Source
To compile the binary yourself with Cargo:
```bash
cargo build --release
```
The compiled binary will be placed at `target/release/daily-news`.

### Running Unit Tests
```bash
cargo test
```
Runs the full suite of unit tests verifying URL canonicalization, deterministic hashing, scoring weights, and storage persistence.

---

## 4. API Endpoints Reference

All endpoints are mounted under the Toolcase Gateway namespace prefix `/custom/daily-news/*`:

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/custom/daily-news/health` | Health check, module version, and active runtime engine (`rust-native`) |
| `GET` | `/custom/daily-news/models` | OpenAI-compatible model listing (`daily-news`) |
| `POST` | `/custom/daily-news/chat/completions` | Formatted daily news summary in chat format |
| `GET` | `/custom/daily-news/ui` or `/` | Interactive responsive HTML status dashboard |
| `POST` | `/custom/daily-news/api/v1/discover` | Tiered content discovery returning ranked candidates and top-level `count` |
| `GET` | `/custom/daily-news/api/v1/candidates` | Returns currently ranked and scored candidates |
| `GET` | `/custom/daily-news/api/v1/feed` | Raw ingested feed items before scoring |
| `POST` | `/custom/daily-news/api/v1/consume` | Marks candidates as consumed/processed to avoid re-serving |
| `GET` | `/custom/daily-news/api/v1/config` | Read active configuration |
| `POST` | `/custom/daily-news/api/v1/config` | Update configuration parameters dynamically |

---

## 5. Query String Parameters & Dual-Link Schema

All routes support query string parameters as well as JSON body parameters:

```bash
# Query string parameter
curl -s -X POST "http://127.0.0.1:28130/custom/daily-news/api/v1/discover?limit=15"

# JSON body parameter
curl -s -X POST http://127.0.0.1:28130/custom/daily-news/api/v1/discover \
  -H "Content-Type: application/json" \
  -d '{"limit": 15}'
```

### Response Schema with Top-Level `count`
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
  ]
}
```

### Understanding the Dual Links
- **`canonical_url`**: The destination article or project website, stripped of all tracking parameters (`utm_*`, `fbclid`, `gclid`, `ref`).
- **`discussion_url`**: The community comment thread or forum post (e.g. Hacker News item), allowing users to browse discussion and context.

---

## 6. Deterministic 6-Axis Scoring

Candidates are ranked using a multi-factor formula:

$$\text{Score} = (W_F \times F) + (W_A \times A) + (W_D \times D) + (W_V \times V) + (W_I \times I) + (W_{\text{Fe}} \times \text{Fe})$$

| Axis | Weight | Criteria |
| :--- | :--- | :--- |
| **Freshness ($F$)** | 25% | $1.0$ for $<12\text{h}$, decays linearly to $0.0$ at $48\text{h}$. |
| **Publisher Authority ($A$)** | 20% | Tier-1 AI labs ($1.0$), HN/TechCrunch ($0.70$), default ($0.50$). |
| **Fact Density ($D$)** | 15% | Content length, technical density, code snippets. |
| **Visual Potential ($V$)** | 15% | Discussion link presence, media richness. |
| **Audience Interest ($I$)** | 15% | Community upvotes, comment velocity. |
| **Feasibility ($\text{Fe}$)** | 10% | Reproducibility and open repo/API availability ($1.0$). |

---

## 7. Configuration Reference

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
