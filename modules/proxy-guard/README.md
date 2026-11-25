# Proxy Guard & Traffic Optimizer

> **Native Rust proxy traffic optimizer & bandwidth saver for Toolcase Gateway.**  
> Intercepts high-bandwidth assets, ad networks, telemetry beacons, and tracking scripts before they reach paid upstream proxies, reducing proxy data consumption by **60% to 95%** per automation flow.

---

## 1. Problem Statement: Why Does 1 Flow Consume >16 MB to 30 MB?

When running automated browser workflows (Playwright, Puppeteer, Selenium), scrapers, or mobile clients through paid residential/mobile proxies (often costing $2 – $15 / GB), bandwidth costs add up exponentially:

| Traffic Component | Typical Size | Critical for Flow? | Action by Proxy Guard |
| :--- | :--- | :--- | :--- |
| **Browser Background Bloat** (`edgedl.me.gvt1.com` Widevine, `openh264`, update) | **9 MB – 20 MB** | ❌ No | **Blocked (204 No Content)** |
| **Heavy Video & Media** (`.mp4`, `.webm`, hero assets) | **5 MB – 25 MB** | ❌ No | **Blocked (204 No Content)** |
| **Analytics & Tag Managers** (GTM, GA4, Meta Pixel) | **500 KB – 1.5 MB** | ❌ No | **Mocked (Harmless Stub JS)** |
| **Session Recording** (Hotjar, Microsoft Clarity, FullStory) | **1 MB – 3 MB** | ❌ No | **Blocked (204 / Dropped WS)** |
| **Error Telemetry** (Sentry, Datadog RUM, New Relic) | **300 KB – 1 MB** | ❌ No | **Blocked (Empty JSON / 204)** |
| **Custom Web Fonts** (`.woff2`, `.woff`, `.ttf`) | **800 KB – 2 MB** | ❌ No | **Blocked (204 No Content)** |
| **Core Document HTML & Essential JSON APIs** | **50 KB – 500 KB** | ✅ **YES** | **Forwarded to Upstream Proxy** |

**Result**: A flow that previously burned **16 MB – 30 MB** is compressed down to **< 1 MB – 6 MB**, saving **60% to 95% of proxy bandwidth and cost**!

---

## 2. Architecture & Clean Architecture Layers (v1.3.1)

`proxy-guard` is authored in **100% Native Rust** following Clean Architecture and Domain-Driven Design (DDD) principles:

```text
modules/proxy-guard/
├── Cargo.toml          # Single source of truth (v1.3.1)
├── module.json         # Declarative gateway manifest & MCP schemas
├── run.sh              # Sub-millisecond precompiled binary launcher
├── proxy-guard         # Native prebuilt release binary (635 KB)
├── config.example.json # Gateway configuration template
├── src/
│   ├── domain/         # Pure business rules, entities, and value objects
│   │   ├── rules.rs    # Domain filtering rules and actions
│   │   ├── proxy.rs    # Upstream proxy entities, parsing, and rotation strategies
│   │   └── mod.rs      # Domain re-exports
│   ├── ports.rs        # Inbound and Outbound port traits
│   ├── service.rs      # Application orchestrator service
│   ├── rules_engine/   # Domain rules matcher & catalog
│   │   ├── catalog/    # Static catalogs: analytics, advertising, media_fonts, browser_noise
│   │   └── mod.rs      # Engine dispatcher & pre-sorted matching
│   ├── request_optimizer/ # Hop-by-hop & tracking query rewriter
│   │   ├── constants.rs# Static query and header stripped lists
│   │   └── mod.rs      # Header rewriting & RFC 9110 compression negotiator
│   ├── proxy_pool.rs   # Upstream multi-proxy pool manager (rotation & failover)
│   ├── proxy_server/   # Multithreaded HTTP/CONNECT interceptor with SOCKS5 & SSRF protection
│   │   ├── http.rs     # Plain HTTP request interception & origin-form parsing
│   │   ├── tunnel.rs   # HTTPS CONNECT tunneling & idle-timeout socket splicing
│   │   ├── utils.rs    # SSRF verification & RFC 1928 SOCKS5 handshake
│   │   └── mod.rs      # Server loop & atomic RAII connection guard
│   ├── mock_factory.rs # Synthetic mock generation (prevents client JS crashes)
│   ├── script_generator.rs # Code generator for Playwright, Puppeteer, mitmproxy
│   ├── storage.rs      # File & In-Memory storage with private 0o600 permissions
│   ├── handlers/       # Interface adapters (routes & dispatcher)
│   ├── rpc.rs          # stdio-stream framing helpers
│   └── mcp.rs          # Model Context Protocol tools adapter
└── tests/              # Automated integration test suite (57 tests)
```

> [!NOTE]
> Detailed integration guide available at [docs/PROXY_GUARD_INTEGRATION.md](../../docs/PROXY_GUARD_INTEGRATION.md).

---

## 3. Deployment Modes

### Mode A: Standalone Intercepting HTTP/CONNECT Proxy
Run Proxy Guard on a local port (e.g. `127.0.0.1:3128`) chained to your upstream paid proxy:
```bash
./proxy-guard --proxy-port 3128 --upstream "http://user:pass@pr.oxylabs.io:7777"
```

Configure your browser or scraper to use `http://127.0.0.1:3128` as its proxy:
```bash
curl -x http://127.0.0.1:3128 https://example.com
```

### Mode B: Upstream Multi-Proxy Pool with Dynamic Rotation
Proxy Guard manages a pool of paid upstream proxies with automatic rotation:
- **Round-Robin**: Distributes load equally across proxies.
- **Sticky Session**: Ties `X-Proxy-Session-Id` header to a dedicated proxy until failure.
- **Lowest Failures**: Dynamically routes through the healthiest proxy in the pool.
- **Random**: Randomly selects a proxy from the pool.

Add proxies via API:
```bash
curl -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/add \
  -H "Content-Type: application/json" \
  -d '{"url": "http://user:pass@pr.oxylabs.io:7777", "name": "oxylabs-1", "country": "US"}'
```

### Mode C: Toolcase Gateway & MCP for AI Agents
Integrated out of the box with Toolcase Gateway via `stdio-stream`:
- Model Context Protocol tools:
  - `proxy-guard__evaluate_url`: Inspect any URL and check whether it will be blocked, mocked, or forwarded.
  - `proxy-guard__optimize_flow`: Analyze an entire batch of requests / HAR 1.2 log entries and calculate exact bandwidth & dollar savings.
  - `proxy-guard__add_upstream_proxy`: Dynamically add upstream proxies to the pool.
  - `proxy-guard__list_upstream_proxies`: List registered proxies with credentials securely masked.
  - `proxy-guard__remove_upstream_proxy`: Remove an upstream proxy by ID.
  - `proxy-guard__set_rotation_strategy`: Set rotation strategy (`round_robin`, `sticky_session`, `lowest_failures`, `random`).
  - `proxy-guard__generate_script`: Output ready-to-run automation interceptors.
  - `proxy-guard__get_metrics`: View real-time bandwidth and dollar savings.
  - `proxy-guard__configure_rules`: Toggle categories or add custom whitelist/blacklist entries.

### Playwright & Headless Browser Integration
For full integration recipes (TypeScript, Python, CI `playwright.config.ts`, route interception, and proxy rotation), refer to the dedicated guide:
👉 [**Playwright Integration Guide**](../../docs/PLAYWRIGHT_INTEGRATION.md)

---

## 4. Verification & Tests

Run the full integration test suite:
```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

All 57 integration tests pass with zero warnings, zero `unsafe`, and zero host dependencies.
