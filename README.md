# Toolcase Gateway Modules Storage Repository

Welcome to the **Toolcase Gateway Modules** repository.

This repository serves as the centralized storage and distribution catalog for modular out-of-process provider extensions, adapters, and tools for [Toolcase Gateway](https://github.com/giauphan/toolcase-gateway).

---

## 1. Repository Structure

All standalone feature modules reside under the [`modules/`](modules/) directory. Following the **Rust-first top-priority architecture**, each module is a self-contained, native Rust crate with its own manifest, prebuilt binary, clean architecture source layout, and test suite:

```text
gateway-modules/
├── README.md                 # Repository overview and module index
├── AGENTS.md                 # Architecture, Rust priority, and quality guidelines
├── .gitignore
└── modules/                  # Feature modules directory
    ├── daily-news/           # Daily News & Content Discovery Service (Native Rust)
    │   ├── Cargo.toml        # Rust package manifest and dependencies
    │   ├── module.json       # Declarative manifest (ID, namespace, capabilities)
    │   ├── run.sh            # Standard executable entrypoint launcher
    │   ├── daily-news        # Prebuilt release binary
    │   ├── config.example.json
    │   ├── README.md         # Module documentation and API reference
    │   ├── src/              # Clean Architecture & DDD implementation
    │   │   ├── main.rs       # stdio-stream framing and RPC dispatch
    │   │   ├── domain.rs     # Pure entities, hashes, and validation rules
    │   │   ├── ports.rs      # Inbound and outbound trait abstractions
    │   │   ├── service.rs    # Domain orchestration service
    │   │   ├── fetcher.rs    # Multi-source live feed adapter
    │   │   ├── redis.rs      # Redis stream / queue adapter
    │   │   ├── scoring.rs    # 6-axis scoring engine
    │   │   └── ui.rs         # Embedded dashboard renderer
    │   └── tests/            # Automated integration tests
    │
    └── <future-module>/      # Future feature modules (native Rust)
        ├── Cargo.toml
        ├── module.json
        ├── run.sh
        ├── src/
        └── tests/
```

---

## 2. Available Feature Modules

| Module ID | Name | Version | Namespace | Capabilities | Path |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`daily-news`** | **Daily News & Content Discovery Service** | `1.3.0` | `/custom/daily-news` | `news`, `content_discovery`, `chat_completions`, `models`, `redis` | [`modules/daily-news`](modules/daily-news) |
| **`facebook-suite`** | **Meta Business Suite / Facebook Creator Studio Adapter** | `1.0.0` | `/custom/facebook` | `studio`, `composer`, `upload`, `publish`, `reels`, `stories`, `auto_translate` | [`modules/facebook-suite`](modules/facebook-suite) |
| **`proxy-guard`** | **Proxy Guard & Traffic Optimizer** | `1.16.0` | `/custom/proxy-guard` | `traffic_optimization`, `adblock`, `proxy_interception`, `analytics_filtering`, `bandwidth_saving`, `mcp` | [`modules/proxy-guard`](modules/proxy-guard) |

### Module Highlights: `proxy-guard`
- **Native Rust Proxy Bandwidth Saver**: Cuts paid proxy traffic by 70%–95% per flow by intercepting heavy media, tracking beacons, ads, and telemetry at the early `HTTP CONNECT` stage (zero MITM, preserving client TLS JA3/JA4 fingerprints).
- **Domain Split Routing & Sticky Sessions**: Routes traffic cleanly across `Reject`, `Direct` (0 GB paid proxy), and named upstream proxy pools (`residential`, `static_isp_us`, etc.) with TTL sticky sessions and HTTP 407 quarantine caching.
- **Interactive Dashboard UI (`/custom/proxy-guard/ui`)**: Self-contained zero-CDN web dashboard featuring a sticky `⚡ Actions ▾` menu (strategy/default-route switchers, auto-refresh, metrics export/reset), 4 inline SVG charts (request disposition, bandwidth saved vs. upstream TX/RX, per-pool traffic, 407 health), one-click proxy quarantine restore, and live split-routing rule controls.
- **Non-Crashing Synthetic Mocks**: Injects harmless stub JavaScript for Google Analytics, GTM, Meta Pixel, and TikTok so client automation scripts never crash.
- **Dual Mode Operation & Script Generators**: Functions as both an in-process MCP module for AI agents and a standalone multithreaded HTTP/CONNECT intercepting proxy listener, with persistent-context Playwright, Puppeteer, and mitmproxy script generators.


### Module Highlights: `facebook-suite`
- **1-Click HAR Ingestion**: Seamlessly extracts session cookies, CSRF tokens (`fb_dtsg`), user ID (`__user`), actor ID (`av`), and targeting parameters from `download-facebook.har`.
- **Resumable Chunked Video Upload (rupload)**: Multi-partition chunked streaming upload with offset reconciliation and integrity checks for large video publishing.
- **Reels & Stories Publishing**: Direct GraphQL and Graph Video API integration supporting title, description, thumbnail, and scheduling.
- **Automated Video Dubbing & Translation**: Default multi-language auto-dubbing metadata support across 14 languages.
- **Embedded Web UI**: Self-contained configuration and status dashboard at `/custom/facebook/ui`.
- **Diagnostic Health Probing**: Supports `/health` probing (`status: "healthy"`) for zero-downtime dashboard diagnostic monitoring.

### Module Highlights: `daily-news`
- **Native Rust Engine**: Zero-Python runtime overhead, sub-millisecond invocation, and self-contained static execution.
- **Multi-Source Ingestion**: Ingests breaking updates from Hacker News, GitHub Trending/Releases, Product Hunt, Reddit, and ArXiv.
- **Redis Queue / Stream Provider**: Ingests articles directly from Redis Streams (`XREVRANGE`) or Redis Lists (`LRANGE`) via configurable `redis://` connection URL.
- **Official Specification Feeds**: Ingests official documentation and specs from `modelcontextprotocol.io`, `docs.anthropic.com`, `docs.github.com`, and `git-scm.com`.
- **SSRF Safety & URL Canonicalization**: Bounded HTTPS checking, loopback/private IP blocking, and tracking parameter stripping (`utm_*`, `fbclid`).
- **Deterministic Deduplication**: Computes SHA-256 `title_hash` and `content_hash` to eliminate duplicates.
- **Multi-Axis Scoring Engine**: Ranks content along six axes (freshness, publisher authority, fact density, visual potential, audience interest, feasibility).
- **Video Flow Compatibility**: Generates full 6-beat engineering storyboard fields (`symptom`, `root_cause`, `remedy`, `evidence_excerpt`, `hook`, `viewer_question`, `source_snapshot`, `snapshot_sha256`).
- **Tiered Discovery**: Priority cascading (Tier 1 Curated Specs $\to$ Tier 2 Automated Feeds $\to$ Tier 3 Fail-Closed).
- **Topic & Date-Range Filtering**: Supports filtering candidates by topic families (`agent_tool_use`, `context_memory`, etc.) and calendar date ranges (`from_date` / `to_date`).

---

## 3. How to Add a New Feature Module (Rust-First Standard)

All new feature modules MUST top-prioritize Rust as their implementation language:

1. Initialize a dedicated Rust binary crate under `modules/<your-module-id>/`:
   ```bash
   cargo new --bin modules/my-feature-module
   cd modules/my-feature-module
   ```
2. Define the declarative manifest `module.json`:
   ```json
   {
     "id": "my-feature-module",
     "name": "My Feature Module",
     "version": "1.0.0",
     "namespace": "/custom/my-feature",
     "entrypoint": "run.sh",
     "protocol": "stdio-stream",
     "capabilities": ["models", "chat_completions"]
   }
   ```
3. Provide the executable launcher `run.sh` (`chmod +x run.sh`):
   ```bash
   #!/bin/sh
   SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
   [ -f "$SCRIPT_DIR/my-feature-module" ] && chmod +x "$SCRIPT_DIR/my-feature-module" 2>/dev/null
   if [ -x "$SCRIPT_DIR/my-feature-module" ]; then
     exec "$SCRIPT_DIR/my-feature-module" "$@"
   fi
   exec cargo run --release --manifest-path "$SCRIPT_DIR/Cargo.toml" -- "$@"
   ```
4. Implement the `stdio-stream` JSON-RPC handler in Rust reading requests from `stdin` and returning responses on `stdout`.
5. Precompile the release binary:
   ```bash
   cargo build --release
   cp target/release/my-feature-module ./my-feature-module
   chmod +x ./my-feature-module
   ```
6. Add unit tests under `tests/` and verify that all tests pass (`cargo test`).
7. Commit and push your changes to this repository.

---

## 4. Installation and Upgrade in Toolcase Gateway

Modules in this repository can be installed or upgraded into Toolcase Gateway via the web dashboard at `http://127.0.0.1:28130/dashboard` or via the gateway REST API:

### Install Module
```bash
curl -X POST http://127.0.0.1:28130/api/v1/dashboard/modules/install \
  -H "Content-Type: application/json" \
  -d '{
    "source": "https://github.com/giauphan/gateway-modules/tree/main/modules/facebook-suite",
    "id": "facebook-suite"
  }'
```

### Upgrade Module
```bash
curl -X POST http://127.0.0.1:28130/api/v1/dashboard/modules/upgrade \
  -H "Content-Type: application/json" \
  -d '{
    "id": "facebook-suite",
    "source": "https://github.com/giauphan/gateway-modules/tree/main/modules/facebook-suite"
  }'
```

