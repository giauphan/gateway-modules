# Proxy Guard & Traffic Optimizer

> **Native Rust proxy traffic optimizer & bandwidth saver for Toolcase Gateway.**  
> Intercepts high-bandwidth assets, ad networks, telemetry beacons, and tracking scripts before they reach paid upstream proxies, reducing proxy data consumption by **90% to 95%** per automation flow.

---

## 1. Problem Statement: Why Does 1 Flow Consume >30 MB?

When running automated browser workflows (Playwright, Puppeteer, Selenium), scrapers, or mobile clients through paid residential/mobile proxies (often costing $2 – $15 / GB), bandwidth costs add up exponentially:

| Traffic Component | Typical Size | Critical for Flow? | Action by Proxy Guard |
| :--- | :--- | :--- | :--- |
| **Heavy Video & Media** (`.mp4`, `.webm`, hero assets) | **15 MB – 25 MB** | ❌ No | **Blocked (204 No Content)** |
| **Analytics & Tag Managers** (GTM, GA4, Meta Pixel) | **500 KB – 1.5 MB** | ❌ No | **Mocked (Harmless Stub JS)** |
| **Session Recording** (Hotjar, Microsoft Clarity, FullStory) | **1 MB – 3 MB** | ❌ No | **Blocked (204 / Dropped WS)** |
| **Error Telemetry** (Sentry, Datadog RUM, Segment) | **300 KB – 1 MB** | ❌ No | **Blocked (Empty JSON / 204)** |
| **Custom Web Fonts** (`.woff2`, `.woff`, `.ttf`) | **800 KB – 2 MB** | ❌ No | **Blocked (204 No Content)** |
| **Ad Exchanges** (DoubleClick, Criteo, Taboola) | **1 MB – 3 MB** | ❌ No | **Blocked (204 No Content)** |
| **Core Document HTML & Essential JSON APIs** | **50 KB – 300 KB** | ✅ **YES** | **Forwarded to Upstream Proxy** |

**Result**: A flow that previously burned **32 MB** is compressed down to **< 300 KB**, achieving a **98%+ reduction in proxy bandwidth and cost**!

---

## 2. Architecture & Clean Architecture Layers

`proxy-guard` is authored in **100% Native Rust** following Clean Architecture and Domain-Driven Design (DDD) principles:

```text
modules/proxy-guard/
├── Cargo.toml          # Single source of truth (v1.0.0)
├── module.json         # Declarative gateway manifest & MCP schemas
├── run.sh              # Sub-millisecond precompiled binary launcher
├── proxy-guard         # Native prebuilt release binary (485 KB)
├── config.example.json # Gateway configuration template
├── src/
│   ├── domain.rs       # Pure business rules, entities, and value objects
│   ├── ports.rs        # Inbound and Outbound port traits
│   ├── service.rs      # Application orchestrator service
│   ├── rules_engine.rs # Domain rules matcher & built-in catalog
│   ├── mock_factory.rs # Synthetic mock generation (prevents client JS crashes)
│   ├── script_generator.rs # Code generator for Playwright, Puppeteer, mitmproxy
│   ├── storage.rs      # In-memory & JSON state persistence adapter
│   ├── proxy_server.rs # Native multithreaded HTTP/CONNECT interceptor
│   ├── handlers/       # Interface adapters (routes & dispatcher)
│   ├── rpc.rs          # stdio-stream framing helpers
│   └── mcp.rs          # Model Context Protocol tools adapter
└── tests/              # Automated integration tests
```

### Non-Crashing Synthetic Mocks
Unlike crude network blockers that cause web pages to throw `ReferenceError: dataLayer is not defined` or `TypeError: window.gtag is not a function`, Proxy Guard injects **harmless stub mocks**:
- **GTM / Google Analytics**: Returns HTTP 200 with `window.dataLayer = window.dataLayer || []; window.gtag = function(){}; window.ga = function(){};`
- **Meta / Facebook Pixel**: Returns HTTP 200 with `window.fbq = function(){}; window._fbq = window.fbq;`
- **TikTok Pixel**: Returns HTTP 200 with `window.ttq = { track: function(){}, page: function(){} };`
- **Tracking Pixels**: Returns standard 43-byte transparent 1x1 GIF (`image/gif`).
- **Telemetry Beacons**: Returns HTTP 204 No Content or `{}`.

---

## 3. Deployment Modes

### Mode A: Browser-Level Zero-Bandwidth Interceptor (Client Script)
Generate interceptor code using the module and paste it directly into your Playwright or Puppeteer automation flow. Blocked requests are cancelled directly inside the browser process—**0 bytes ever leave your machine or hit your proxy**.

Export scripts via MCP or HTTP:
- Playwright TypeScript: `GET /custom/proxy-guard/scripts/playwright_ts`
- Playwright Python: `GET /custom/proxy-guard/scripts/playwright_py`
- Puppeteer: `GET /custom/proxy-guard/scripts/puppeteer_ts`
- mitmproxy: `GET /custom/proxy-guard/scripts/mitmproxy_py`

### Mode B: Standalone Intercepting HTTP/CONNECT Proxy
Run Proxy Guard on a local port (e.g. `127.0.0.1:3128`) chained to your upstream paid proxy:
```bash
./proxy-guard --proxy-port 3128 --upstream "http://user:pass@pr.oxylabs.io:7777"
```
Configure your client/browser to use `http://127.0.0.1:3128` as its HTTP/HTTPS proxy:
```bash
curl -x http://127.0.0.1:3128 https://example.com/checkout
```
- **Plain HTTP**: Evaluates URL. Blocked requests receive immediate local synthetic mocks (0 bytes upstream). Allowed requests forward to upstream.
- **HTTPS CONNECT**: Evaluates `CONNECT host:443`. If the domain is an ad/tracker/media domain, Proxy Guard immediately responds with `403 Forbidden` and terminates the connection before TLS handshakes occur, completely eliminating upstream bandwidth consumption.

### Mode C: Toolcase Gateway & MCP for AI Agents
Integrated out of the box with Toolcase Gateway via `stdio-stream`:
- Model Context Protocol tools:
  - `proxy-guard__evaluate_url`: Inspect any URL and check whether it will be blocked, mocked, or forwarded.
  - `proxy-guard__optimize_flow`: Analyze an entire batch of requests / HAR log and calculate exact bandwidth & dollar savings.
  - `proxy-guard__generate_script`: Output ready-to-run automation interceptors.
  - `proxy-guard__get_metrics`: View real-time bandwidth and dollar savings.
  - `proxy-guard__configure_rules`: Toggle categories or add custom whitelist/blacklist entries.

---

## 4. API Reference

### `POST /custom/proxy-guard/evaluate`
```json
{
  "url": "https://www.google-analytics.com/analytics.js",
  "method": "GET",
  "resource_type": "script"
}
```
**Response (200 OK)**:
```json
{
  "url": "https://www.google-analytics.com/analytics.js",
  "method": "GET",
  "resource_type": "script",
  "action": "mock_script",
  "matched_rule_id": "ga-analytics-domain",
  "category": "analytics",
  "estimated_bytes_saved": 85000,
  "mock": {
    "status": 200,
    "content_type": "application/javascript; charset=utf-8",
    "body": "/* [Proxy Guard] GA/GTM Stubbed for zero proxy bandwidth */\nwindow.dataLayer = window.dataLayer || [];\nwindow.gtag = window.gtag || function() { (window.dataLayer.push(arguments)); };\nwindow.ga = window.ga || function(cb) { if (typeof cb === 'function') { cb(); } };\nwindow.google_tag_manager = window.google_tag_manager || {};"
  },
  "reason": "Matched rule 'Google Analytics Domain' (Analytics)"
}
```

### `POST /custom/proxy-guard/optimize-flow`
Evaluates a batch array of requests in a workflow:
```json
{
  "requests": [
    { "url": "https://shop.example.com/checkout", "resource_type": "document", "size_bytes": 50000 },
    { "url": "https://shop.example.com/promo.mp4", "resource_type": "media", "size_bytes": 25000000 },
    { "url": "https://www.googletagmanager.com/gtm.js", "resource_type": "script", "size_bytes": 150000 }
  ]
}
```
**Response (200 OK)**:
```json
{
  "total_flow_requests": 3,
  "blocked_requests": 2,
  "forwarded_requests": 1,
  "original_estimated_mb": 24.03,
  "optimized_estimated_mb": 0.05,
  "reduction_percentage": 99.8,
  "cost_saved_usd": 0.0839,
  "decisions": [...]
}
```

---

## 5. Verification & Tests

Run the full integration test suite:
```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```
All tests run with zero warnings and zero dependencies on host runtimes.
