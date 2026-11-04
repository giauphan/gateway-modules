# Daily News & Content Discovery Service Module (`daily-news`)

A high-performance standalone provider module for **Toolcase Gateway**, extracted from OpenMontage's daily news gathering, canonicalization, and tiered ranking pipeline.

The module is **100% Native Rust** with **zero Python dependencies**.

---

## Table of Contents

1. [Features & Capabilities](#1-features--capabilities)
2. [Clean Architecture & File Layout](#2-clean-architecture--file-layout)
3. [Running, Compiling & Testing](#3-running-compiling--testing)
4. [API Endpoints Reference](#4-api-endpoints-reference)
5. [Candidate Contract & 6-Beat Storyboard Evidence](#5-candidate-contract--6-beat-storyboard-evidence)
6. [Dual-Link Resolution Schema](#6-dual-link-resolution-schema)
7. [Deterministic 6-Axis Scoring Engine](#7-deterministic-6-axis-scoring-engine)
8. [Multi-Source Feeds & Redis Stream Ingestion](#8-multi-source-feeds--redis-stream-ingestion)
9. [Configuration Reference](#9-configuration-reference)
10. [Documentation Index](#10-documentation-index)

---

## 1. Features & Capabilities

- **100% Pure Native Rust**: Zero Python runtime, pip dependencies, or virtualenvs required.
- **Unified Query & Body Resolution**: Accepts parameters seamlessly via URL query strings (`?limit=15`, `?from_date=2026-11-01`, `?topics=agent_tool_use`) or JSON request bodies across all routes.
- **Multi-Source News Ingestion**: Ingests breaking stories from Hacker News Firebase API, Reddit (OAuth, public JSON, and Arctic Shift fallback), upstream [NewsNow](https://github.com/newsnext/newsnow) instances (`/api/s?id={source}`), and Redis Streams (`XREVRANGE`) / Lists (`LRANGE`).
- **SSRF Protection & URL Canonicalization**: Enforces strict URL sanitization, blocking private IPv4 ranges (CWE-918) and stripping tracking parameters (`utm_*`, `fbclid`, `gclid`, `ref`, `source`).
- **Dual-Link Resolution**: Disambiguates between the content destination (`canonical_url`) and the community discussion thread (`discussion_url`).
- **6-Beat Storyboard Engineering Contract**: Provides typed domain fields (`hook`, `viewer_question`, `symptom`, `root_cause`, `remedy`, `evidence_excerpt`, `source_snapshot`, `snapshot_sha256`) that plug directly into OpenMontage automated video generation.
- **Deterministic Deduplication & Hashing**: Computes authoritative SHA-256 `title_hash` and `content_hash` signatures.
- **Explainable 6-Axis Scoring**: Computes objective scores across Freshness, Publisher Authority, Fact Density, Visual Potential, Audience Interest, and Feasibility.
- **Tiered Cascade Discovery**: Employs Tier 1 (Curated Queue) -> Tier 2 (Automated Breaking Feed) -> Tier 3 (Fail-Closed Safety).
- **Interactive Dark-Mode UI**: Built-in responsive HTML status dashboard mounted at `/ui` or `/custom/daily-news`.

---

## 2. Clean Architecture & File Layout

Organized strictly according to Clean Architecture and Domain-Driven Design (DDD). Every file remains strictly under the project line ceiling:

```text
modules/daily-news/
├── Cargo.toml          # Rust crate manifest (serde, ureq, sha2, uuid)
├── Cargo.lock          # Locked deterministic dependencies
├── daily-news          # Precompiled standalone ELF binary (stripped native)
├── run.sh              # Standalone executable launcher (auto-chmod + fallback)
├── module.json         # Toolcase Gateway manifest
├── config.example.json # Configuration template
├── README.md           # Master technical guide
├── docs/
│   ├── API_REFERENCE.md              # Complete API endpoints & JSON schemas
│   ├── VIDEO_PIPELINE_INTEGRATION.md # OpenMontage 6-beat storyboard contract
│   └── ARCHITECTURE.md               # Clean architecture & storage design
└── src/
    ├── domain.rs       # Domain Entities, StoryboardEvidence VO, ModuleConfig
    ├── url.rs          # URL canonicalizer, tracking stripper, SHA-256 hasher, SSRF protection
    ├── date.rs         # Date parser, midnight boundaries, epoch calculation
    ├── ports.rs        # Outbound Port Traits (FeedFetcherPort, DiscussionFetcherPort, StoragePort)
    ├── discovery.rs    # Cascading discovery bounds validation, topic matching, fail-closed construction
    ├── scoring.rs      # Domain Service: Deterministic 6-axis scoring & ranker
    ├── service.rs      # Application Layer: Discovery, Consume, and Config use cases
    ├── storage.rs      # Storage Adapters: InMemoryStorageAdapter & FileStorageAdapter
    ├── fetcher.rs      # Network Adapter: Multi-source feed orchestrator
    ├── hn.rs           # Network Adapter: Live Hacker News Firebase API client & comment enrichment
    ├── reddit.rs       # Network Adapter: Reddit OAuth & Public JSON feed provider
    ├── arctic.rs       # Network Adapter: Zero-key Arctic Shift Reddit archive & search fallback
    ├── discussion.rs   # Network Adapter: Discussion thread & comment tree inspector
    ├── redis.rs        # Network Adapter: Native RESP Redis stream & list client
    ├── ui.rs           # Presenter Adapter: Responsive HTML dashboard presenter
    ├── lib.rs          # Library facade and module re-exports
    ├── rpc.rs          # Interface Adapter: JSON-RPC stdio protocol framing & request parsing
    ├── research_readiness.rs # Domain Service: Truthful readiness evaluation, claim citation checking
    ├── source_capture.rs # Network Adapter: SSRF-guarded primary source capture & SHA-256 snapshot storage
    └── main.rs         # Frameworks & Drivers: stdio-stream JSON-RPC composition root & event loop
```

For complete architectural details, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## 3. Running, Compiling & Testing

### Running Standalone
```bash
./run.sh
```
When Toolcase Gateway starts, `./run.sh` immediately executes the precompiled standalone native binary (`./daily-news`), falling back to `cargo run --release` only if the binary is absent.

### Compiling from Source
```bash
cargo build --release
```
Places the optimized binary in `target/release/daily-news`. Copy to the module root to update:
```bash
cp target/release/daily-news ./daily-news && chmod +x ./daily-news
```

### Running Test Suite
```bash
cargo test
```
Executes all 32 unit and integration tests across discovery tiers, normalizers, URL canonicalizers, Redis parsers, and storage lifecycle.

---

## 4. API Endpoints Reference

All endpoints are mounted under the Toolcase Gateway namespace prefix `/custom/daily-news/*`:

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/custom/daily-news/health` | Health check, module version, and active runtime engine (`rust-native`) |
| `GET` | `/custom/daily-news/models` | OpenAI-compatible model listing (`daily-news`) |
| `POST` | `/custom/daily-news/chat/completions` | Formatted daily news summary in chat completion format |
| `GET` | `/custom/daily-news/ui` or `/` | Interactive responsive HTML status dashboard |
| `POST` | `/custom/daily-news/api/v1/discover` | Tiered content discovery returning ranked candidates and top-level `count` |
| `GET` | `/custom/daily-news/api/v1/candidates` | Returns currently ranked and scored candidates |
| `POST` | `/custom/daily-news/api/v1/candidates` | Normalizes and scores custom injected raw items |
| `GET` | `/custom/daily-news/api/v1/feed` | Raw ingested feed items before scoring |
| `POST` | `/custom/daily-news/api/v1/discussion` | Structured comment tree & thread discussion inspection (Reddit / HN) |
| `POST` | `/custom/daily-news/api/v1/consume` | Marks candidates as consumed/processed to avoid re-serving |
| `GET` | `/custom/daily-news/api/v1/config` | Read active configuration |
| `POST` | `/custom/daily-news/api/v1/config` | Update configuration parameters dynamically |

For complete request/response schemas, query parameters, and error codes, see [docs/API_REFERENCE.md](docs/API_REFERENCE.md).

---

## 5. Candidate Contract & 6-Beat Storyboard Evidence

The candidate model contains first-class, typed domain fields representing the **6-Beat Engineering Storyboard** utilized by OpenMontage for vertical video generation:

```json
{
  "candidate_id": "cand_eb6f9d5de3c4d7e1",
  "canonical_url": "https://docs.anthropic.com/agent-tools",
  "discussion_url": "https://news.ycombinator.com/item?id=50334704",
  "title": "Model Context Protocol Tool Schema Update",
  "publisher": "Anthropic",
  "topic_family": "agent_tool_use",
  "hook": "Is your coding agent choking on 50 MCP tool definitions?",
  "viewer_question": "How do you scope tool schemas dynamically?",
  "symptom": "Agent context window overwhelmed by thousands of lines of unused tool schemas.",
  "root_cause": "Eager registration dumps all schemas into initial prompt.",
  "remedy": "Implement two-tier tool resolution and lazy schema loading.",
  "evidence_excerpt": "Tool schemas may now be dynamically requested via search.",
  "source_snapshot": "artifacts/mcp_spec.txt",
  "snapshot_sha256": "2bad0815c06b65dbe5d27759f1e92852a96473001e0daf2f98f32ced70444d72"
}
```

For full details on the video generation pipeline, see [docs/VIDEO_PIPELINE_INTEGRATION.md](docs/VIDEO_PIPELINE_INTEGRATION.md).

---

## 6. Dual-Link Resolution Schema

To support video generation descriptions and community engagement:
- **`canonical_url`**: The cleaned destination article or official documentation, stripped of tracking queries (`utm_*`, `fbclid`, `gclid`, `ref`).
- **`discussion_url`**: The community comment thread (e.g. Hacker News item), providing public sentiment and developer context.

---

## 7. Deterministic 6-Axis Scoring Engine

Candidates are ranked using a multi-factor formula:

$$\text{Score} = (W_F \times F) + (W_A \times A) + (W_D \times D) + (W_V \times V) + (W_I \times I) + (W_{\text{Fe}} \times \text{Fe})$$

| Axis | Weight | Calculation Criteria |
| :--- | :--- | :--- |
| **Freshness ($F$)** | 30% | $1.0$ for $<2\text{h}$, decays linearly to $0.5$ at $12\text{h}$, $0.2$ at $24\text{h}$, and $0.1$ at $>24\text{h}$. |
| **Publisher Authority ($A$)** | 20% | Tier-1 AI labs & newsrooms ($1.0$), HN/TechCrunch/Verge ($0.80$), known ($0.50$), unknown ($0.20$). |
| **Fact Density ($D$)** | 20% | Word count tiers (>400: $1.0$, >200: $0.8$, >50: $0.5$) with quotes ($+0.1$) & digits ($+0.1$) bonuses. |
| **Visual Potential ($V$)** | 15% | Base ($0.30$) + image/discussion ($+0.30$) + visual keywords (chart, graph, demo, release, prototype: $+0.20$ or $+0.40$). |
| **Audience Interest ($I$)** | 10% | Technical/AI taxonomy keywords ($0.10, 0.75, 1.0$) blended 70/30 with engagement metrics (upvotes + 2×comments). |
| **Feasibility ($\text{Fe}$)** | 5% | Reproducibility, open repository availability, and valid actionable contract ($1.0$). |

---

## 8. Multi-Source Feeds & Redis Stream Ingestion

The module ingests from four primary feed sources:
1. **Hacker News Firebase API**: Top AI stories and discussion threads.
2. **Reddit**: Native OAuth, public JSON, and zero-key Arctic Shift fallback client.
3. **Upstream NewsNow Instances ([newsnext/newsnow](https://github.com/newsnext/newsnow))**: Connects to upstream NewsNow services via `GET /api/s?id={source}` for hotlists across GitHub, Product Hunt, Hacker News, and other regional feeds.
4. **Redis Streams (`XREVRANGE`) & Lists (`LRANGE`)**: Real-time queue ingestion via native, non-panicking synchronous RESP client over `std::net::TcpStream`.

To enable Redis ingestion, configure in `config.json`:
```json
{
  "redis_enabled": true,
  "redis_url": "redis://127.0.0.1:6379/0",
  "redis_key": "news:incoming",
  "redis_type": "stream"
}
```

---

## 9. Configuration Reference

Dynamic configuration parameters stored in `config.json`:

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

## 10. Documentation Index

- [Complete API & Protocol Reference](docs/API_REFERENCE.md)
- [OpenMontage Video Generation Pipeline Integration](docs/VIDEO_PIPELINE_INTEGRATION.md)
- [Architecture & Technical Design](docs/ARCHITECTURE.md)
