# Proxy Guard & Multi-Proxy Gateway Integration Guide

> **Native Rust proxy interceptor, multi-upstream rotation pool, and 60%+ to 95% bandwidth cost reducer for Toolcase Gateway.**  
> Optimized for automated scrapers, headless browsers (Playwright, Puppeteer), and AI agents.

---

## 1. System Architecture & Flow Topology

When running browser automation or scraping tools through paid residential/mobile proxies ($3 – $15 / GB), browsers waste up to 90% of their bandwidth on background updater binaries, video codecs, analytics, and font files.

Proxy Guard sits between your automation tools and the paid proxy providers as a local intercepting gateway:

```text
┌─────────────────────────────────────────────────────────────┐
│                      AUTOMATION TOOLS                       │
│  Playwright (TS/Py) │ Puppeteer │ cURL │ Python Requests    │
└──────────────────────────────┬──────────────────────────────┘
                               │ HTTP / HTTPS CONNECT traffic
                               ▼
┌─────────────────────────────────────────────────────────────┐
│             TOOLCASE GATEWAY / PROXY GUARD                  │
│                   (127.0.0.1:3128)                          │
│                                                             │
│  1. Request Classifier & 3-Way Domain Split Router          │
│     ├── REJECT / Mock (0 Bytes sent upstream):              │
│     │   ├── Browser Bloat (*.gvt1.com Widevine: ~9.1MB)     │
│     │   ├── OpenH264 Video Codec (~523KB)                   │
│     │   ├── Google Tag Manager / GA4 (Mock JS Stub)         │
│     │   ├── Meta Pixel, Sentry, New Relic (Mock JS Stub)    │
│     │   └── Custom Web Fonts (.woff2, .ttf) & scontent      │
│     ├── DIRECT Bypass (BypassDirect — 0 GB paid proxy):     │
│     │   ├── Public static CDNs (static.xx.fbcdn.net,        │
│     │   │   js.stripe.com, custom add_direct_bypass_domains)│
│     │   └── Preserves original TLS fingerprint & HTTP cache │
│     └── Forward Upstream (Sensitive Domains Only):          │
│         ├── services.sheerid.com, muse.ai, auth.meta.com    │
│         ├── Request Optimizer (strip utm_*, fbclid, sec-ch) │
│         └── RFC 9110 Safe Compression Rewriter              │
│                                                             │
│  2. Multi-Upstream Proxy Pool & Health Manager              │
│     ├── Round-Robin / Random / Lowest-Failures              │
│     ├── Sticky Session Routing (per session_id)             │
│     └── SOCKS5 & HTTP/HTTPS Upstream Tunneling              │
└──────────────────────────────┬──────────────────────────────┘
                               │ Forwarding allowed traffic only
                               ▼
┌─────────────────────────────────────────────────────────────┐
│               PAID UPSTREAM PROXY POOL                      │
│   Oxylabs │ BrightData │ Smartproxy │ SOCKS5 / IPv6 Nodes   │
└──────────────────────────────┬──────────────────────────────┘
                               │ Residential IP
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                     TARGET WEBSITES                         │
│             SheerID │ E-Commerce │ SaaS APIs                │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Multi-Proxy Pool Configuration

Proxy Guard supports managing multiple upstream proxies with automatic rotation, health monitoring, and failover.

### 2.1 Supported Proxy Formats

| Format | Example | Protocol |
| :--- | :--- | :--- |
| **Standard URL with Auth** | `http://user:pass@pr.oxylabs.io:7777` | HTTP / HTTPS |
| **SOCKS5 URL with Auth** | `socks5://user:pass@127.0.0.1:1080` | SOCKS5 (RFC 1928/1929) |
| **Compact Vendor Format** | `192.168.1.50:9000:myuser:mypass` | HTTP |
| **Bracketed IPv6** | `http://user:pass@[2001:db8::1]:8080` | Dual-Stack IPv6 |
| **URL-Encoded Passwords** | `http://user:pass%2Bword%2F123@proxy.io:8080` | Preserves `+`, `/`, `@` |

### 2.2 Adding Proxies via JSON-RPC / HTTP

```bash
# Add an Oxylabs residential proxy tagged for US traffic
curl -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/add \
  -H "Content-Type: application/json" \
  -d '{
    "url": "http://customer-xyz:pass123@pr.oxylabs.io:7777",
    "name": "oxylabs-us-1",
    "country": "US",
    "tags": ["residential", "fast"]
  }'

# Add a backup SOCKS5 proxy
curl -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/add \
  -H "Content-Type: application/json" \
  -d '{
    "url": "socks5://res-user:res-pass@10.10.1.20:1080",
    "name": "backup-socks5",
    "country": "DE",
    "tags": ["backup", "socks5"]
  }'
```

### 2.3 Rotation Strategies (Global & Per-Request)

#### Global Strategy Configuration
You can configure the pool's default rotation strategy on the fly:

```bash
curl -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/strategy \
  -H "Content-Type: application/json" \
  -d '{"strategy": "lowest_failures"}'
```

Available strategies:
- `round_robin` (Default): Distributes requests equally across all enabled, healthy proxies.
- `sticky_session`: Routes requests with the same session identifier (`X-Proxy-Session-Id`) to the same proxy IP. Automatically fails over if that proxy encounters consecutive errors.
- `lowest_failures`: Prioritizes proxies with the highest success rate and lowest error counts.
- `random`: Random selection from healthy proxies in the pool.

#### Per-Request Strategy & Country Selection (v1.5.1+)
Instead of changing the global strategy for all workers, individual requests or browser contexts can override the rotation strategy and filter by country on the fly:

> 📘 **Full API Reference**: For exhaustive field schemas, JSON response payloads, and error codes, see [**API Payload Reference Guide**](./PROXY_GUARD_API_PAYLOAD_REFERENCE.md).

1. **Via HTTP Header (`X-Proxy-Strategy` & `X-Proxy-Country`)**:
   Pass `X-Proxy-Strategy: lowest_failures` and `X-Proxy-Country: VN` in your request or Playwright `extraHTTPHeaders`. Proxy Guard routes the request using that strategy and country filter, then automatically strips the headers before forwarding upstream.

2. **Via REST API (`/proxies/next`)**:
   Query parameters or request body take immediate precedence:
   ```bash
   # POST JSON Body (Recommended)
   curl -s -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/next \
     -H "Content-Type: application/json" \
     -d '{
       "country": "VN",
       "strategy": "lowest_failures",
       "session_id": "profile-99"
     }'

   # Query parameter (GET)
   curl -s "http://127.0.0.1:3128/custom/proxy-guard/proxies/next?country=VN&strategy=lowest_failures"
   ```

   **Payload Fields for `POST /proxies/next`**:
   | Field | Type | Description |
   | :--- | :--- | :--- |
   | `country` | `string` (Optional) | Two-letter ISO country code (`"VN"`, `"US"`, etc.). Case-insensitive. |
   | `strategy` | `string` (Optional) | Balancing mode: `"round_robin"`, `"sticky_session"`, `"lowest_failures"`, `"random"`. |
   | `session_id` | `string` (Optional) | Sticky session key. Binds consecutive requests to the same proxy IP. |
   | `tag` | `string` (Optional) | Tag filter (e.g. `"residential"`, `"mobile"`). |
   | `preferred_id` | `string` (Optional) | Preferred proxy ID (`host:port`) if healthy. |

3. **Via MCP Tool**:
   Call `proxy-guard__get_next_proxy` with arguments:
   ```json
   {
     "country": "VN",
     "strategy": "lowest_failures",
     "session_id": "profile-99"
   }
   ```

---

### 2.4 Managing Proxies in the UI

You can manage and save multi-proxies in two places within the user interface:

#### Method A: Toolcase Gateway Settings UI (Configuration Page)
- **Path**: Navigate to **Modules** > **proxy-guard** > **Configuration** (or open `http://<host>:<port>/modules/proxy-guard/config`).
- **Field**: In the **"Upstream Multi-Proxy Pool (1 per line)"** textarea, paste clean unique proxies (`http://username:password@host:port` or `socks5://username:password@host:port`, one per line — duplicates are automatically deduplicated).
- **Country Acronym & `-cc-<country>`**: Set **"Country Acronym (2-Letter ISO or -cc-XX)"** (`default_country`, e.g. `VN` or `-cc-vn`). Enable **"Append `-cc-<country>` to Username"** (or enter `-cc-vn` directly) to automatically inject `-cc-<country>` into each proxy's username without modifying every line.
- **Rotation Strategy**: Set the default strategy in the **"Proxy Rotation Strategy"** input (`round_robin`, `sticky_session`, `lowest_failures`, `random`).
- **Save**: Click **Save Settings**. Proxy Guard saves the pool to disk (`data/proxies.json`) and instantly reloads the pool into memory.

#### Method B: Interactive Proxy Guard Dashboard
- **Path**: Open `http://<host>:<port>/custom/proxy-guard/ui` in your browser.
- **Features**:
  - Live metrics: Total active proxies, current rotation strategy, Request Reduction %, **Data Optimized (GB Saved)**, **Upstream Used (GB)**, **Data Optimization Efficiency** progress bar, and estimated cost savings in USD.
  - Proxy health table: Protocol, masked auth, host:port, country, tags, success/failure counts, and real-time status badges (`Healthy`, `Recovering`, `Failing`).
  - **➕ Add Multiple Upstream Proxies**: Enter clean `http://username:password@host:port` lines in the multi-line textarea, set the **Country** field (e.g. `VN`, `US`, or `-cc-vn`), optionally check **Append `-cc-<country>` to username**, and click **Add to Proxy Pool**.
  - One-click removal via the **Remove** button on any table row.

---

### 2.5 Empty Pool Handling & Client Fallback (`No healthy upstream proxy available in pool`)

When a fresh gateway instance is deployed, or before upstream proxies have been added, the pool in `data/proxies.json` contains 0 proxies. Calling `GET /custom/proxy-guard/proxies/next` or `POST /custom/proxy-guard/proxies/next` will return HTTP 404:

```json
{
  "error": "No healthy upstream proxy available in pool"
}
```

#### Causes
1. **Empty Pool**: No upstream proxies have been registered via the UI dashboard, Settings config, or API yet.
2. **All Proxies Failing**: Proxies were added, but all of them exceeded consecutive failure thresholds during health probes.
3. **Filter Mismatch**: Requesting a specific country (e.g. `?country=VN`) when no proxies in the pool match that country code.

#### Recommended Solutions & Best Practices
1. **Populate Upstream Proxies**:
   - Add your paid proxy credentials (e.g. Oxylabs, BrightData, Smartproxy, or custom SOCKS5 nodes) using the UI at `http://<gateway-host>/custom/proxy-guard/ui` or `/modules/proxy-guard/config`.
   - Alternatively, register proxies dynamically via API:
     ```bash
     curl -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/add \
       -H "Content-Type: application/json" \
       -d '{"url":"http://user:pass@pr.oxylabs.io:7777","country":"US"}'
     ```
2. **Client-Side Graceful Fallback**:
   Automation scripts (such as Playwright, Puppeteer, or crawler bots) should catch the 404 error and fall back to local proxy settings (e.g., in `config/settings.yaml` or environment variables) so runs never abort unexpectedly:
   ```python
   # Python Requests / Playwright Client Example
   try:
       resp = requests.post(
           "https://gateway.genrostore.com/custom/proxy-guard/proxies/next",
           json={"country": "VN", "strategy": "lowest_failures"},
           timeout=5,
       )
       if resp.status_code == 200:
           proxy_info = resp.json().get("playwright_config")
       else:
           # Fallback to local settings when pool is empty
           proxy_info = local_fallback_proxy()
   except Exception:
       proxy_info = local_fallback_proxy()
   ```
3. **Robust Route Matching (Query String & POST Body)**:
   Proxy Guard normalizes request paths by stripping URL query strings (`/proxies/next?country=VN` → `/proxies/next`) before matching routes. Clients may query the next proxy using either `GET /proxies/next?country=US&strategy=lowest_failures` or `POST /proxies/next` with a JSON payload `{"country":"US","strategy":"lowest_failures"}`.

---

## 3. Bandwidth Optimization & Website Compatibility

Unlike crude blockers that return `403 Forbidden` or drop sockets (causing client-side JavaScript to crash with `Uncaught TypeError: window.dataLayer is undefined`), Proxy Guard uses **Synthetic Mock Generation**:

### 3.1 Decision Breakdown

| Traffic Category | Domains / Patterns | Proxy Guard Action | Bandwidth Saved | Compatibility Impact |
| :--- | :--- | :--- | :---: | :--- |
| **Browser Background Bloat** | `*.gvt1.com`, `ciscobinary.openh264.org`, `update.googleapis.com` | **Block (204 No Content)** | **9.67 MB** | **0% impact**: Internal to browser, websites never call these. |
| **Analytics & Tag Managers** | `googletagmanager.com`, `google-analytics.com` | **Mock JS Stub (200 OK)** | **0.20 MB** | **0% impact**: Provides `dataLayer`, `gtag()`, `ga()` stubs; page scripts run error-free. |
| **Pixels & Error Telemetry** | `connect.facebook.net`, `js-agent.newrelic.com`, `sentry.io` | **Mock JS Stub / Empty JSON** | **0.07 MB** | **0% impact**: Provides `fbq()`, `Sentry.captureException()` stubs. |
| **Web Fonts** | `fonts.gstatic.com`, `fonts.googleapis.com` | **Block (204 No Content)** | **0.11 MB** | **0% impact**: Browser falls back to system fonts (Arial, Roboto); buttons and DOM selectors are identical. |
| **Anti-Fraud Profiling** | `online-metrix.net` (LexisNexis ThreatMetrix) | **Forwarded (Allowed)** | 0 KB | **Preserved**: Required by sensitive identity platforms (e.g. SheerID) to prevent fraud rejection. |
| **Cloudflare Turnstile** | `challenges.cloudflare.com` | **Forwarded (Allowed)** | 0 KB | **Preserved**: Allows bot / automation tools to render and solve Turnstile Captchas cleanly. |

### 3.2 Live Request Audit Logs & Reduction Monitoring (v1.5.0+)

Proxy Guard maintains an in-memory bounded ring buffer (latest 100 entries) capturing real-time network interception events. This enables live debugging, audit compliance, and immediate verification that bloat traffic is being stopped before hitting upstream proxies.

#### Real-Time Metric & Request Reduction Calculation
The system calculates request reduction percentage dynamically:
$$\text{Request Reduction \%} = \frac{\text{Blocked Requests}}{\text{Blocked Requests} + \text{Forwarded Requests}} \times 100$$

#### REST Endpoints for Logs
- **Retrieve Request Logs (`GET /custom/proxy-guard/logs`)**:
  ```bash
  curl -s "http://127.0.0.1:3128/custom/proxy-guard/logs?limit=50"
  ```
  **Response Payload**:
  ```json
  {
    "status": "success",
    "request_reduction_percentage": 78.5,
    "blocked_requests": 157,
    "forwarded_requests": 43,
    "total_requests": 200,
    "logs": [
      {
        "id": 200,
        "timestamp": 1700923050,
        "method": "GET",
        "host": "ciscobinary.openh264.org",
        "path": "/openh264-win64-2e17de1.zip",
        "action": "Blocked (204 No Content)",
        "category": "Browser Bloat (Codecs)",
        "bytes_saved": 535552,
        "is_blocked": true
      },
      {
        "id": 199,
        "timestamp": 1700923049,
        "method": "GET",
        "host": "www.google-analytics.com",
        "path": "/analytics.js",
        "action": "Mocked (JS Stub)",
        "category": "Analytics & Trackers",
        "bytes_saved": 47104,
        "is_blocked": true
      },
      {
        "id": 198,
        "timestamp": 1700923048,
        "method": "POST",
        "host": "api.targetservice.com",
        "path": "/v1/checkout",
        "action": "Forwarded (Upstream)",
        "category": "Core Application Traffic",
        "bytes_saved": 0,
        "is_blocked": false
      }
    ]
  }
  ```

- **Clear In-Memory Audit Logs (`POST /custom/proxy-guard/logs/clear`)**:
  ```bash
  curl -s -X POST http://127.0.0.1:3128/custom/proxy-guard/logs/clear
  ```
  **Response**:
  ```json
  {
    "status": "success",
    "message": "Request logs cleared successfully"
  }
  ```

#### Live UI Dashboard (`/custom/proxy-guard/ui`)
The interactive web dashboard displays:
1. **Request Reduction % Card**: Real-time percentage badge showing how much traffic was filtered out before reaching paid proxies.
2. **Bandwidth & Upstream Usage Cards**: Instant readout of total MB saved vs. upstream data used, alongside USD cost savings.
3. **Live Intercepted Request Logs Table**:
   - Status badges with distinct color-coding: `Blocked (204)` (red/amber), `Mocked (JS Stub)` (purple/indigo), and `Forwarded (Upstream)` (green).
   - Domain, route path, category badge, timestamp, and bytes saved per entry.
   - **One-click "Clear Logs" button** with automatic host/path resolution for reverse proxy environments.

### 3.3 Domain Split Routing, Caching & TLS Fingerprint Strategy (v1.14.0+)

For strict anti-bot platforms such as **`services.sheerid.com`** and **`muse.ai`**, the most effective way to cut paid residential proxy GB consumption is **not just caching**, but **domain-based split routing** so heavy non-sensitive assets never traverse pay-per-GB proxies while sensitive hosts retain their exact original browser TLS fingerprint.

#### Recommended 3-Tier Solution Architecture

| Tier | Approach | GB Saved | TLS Fingerprint | Best For |
| :--- | :--- | :---: | :---: | :--- |
| **Group 1 (Recommended)** | **Domain Split Routing (`REJECT` / `DIRECT` / `Upstream`)** | **70% – 90%+** | **100% Preserved** | Headless browsers & strict anti-bot targets (`services.sheerid.com`, `muse.ai`, `auth.meta.com`) |
| **Group 2** | **HTTP Client-Layer Caching** (`hishel`, `requests-cache`) | **50% – 85%** | **100% Preserved** | Pure REST API tools with repeated lookups (school/org search, metadata) |
| **Group 3 (Use with Care)** | **MITM Caching Proxy** (`mitmproxy-anycache`, `python-mitmcache`) | **60% – 80%** | **Altered (OpenSSL)** | Static/CDN hosts only — **must** exclude sensitive hosts via `--ignore-hosts` |

---

#### Group 1: Domain Split Routing (70–90% GB Saved, Intact TLS Fingerprint)

Route only sensitive domains through paid upstream proxies; **reject** trackers/fonts/ads and route public static CDNs **`DIRECT`** (0 GB paid proxy):

| Domain / Host Pattern | Recommended Route | Proxy Guard Action | Paid Proxy GB |
| :--- | :--- | :--- | :---: |
| `services.sheerid.com` | Paid Residential Proxy (e.g. DataImpulse / Oxylabs) | `ForwardUpstream` | Minimal (API/HTML only) |
| `muse.ai`, `auth.muse.ai`, `api.muse.ai` | Static US ISP / Residential Proxy | `ForwardUpstream` | Minimal (API/HTML only) |
| `auth.meta.com`, `m.stripe.com`, `challenges.cloudflare.com` | Paid Upstream Proxy | `ForwardUpstream` | Minimal |
| `static.xx.fbcdn.net`, `js.stripe.com`, public JS/CSS CDNs | **`DIRECT`** (Local network, SSRF-checked) | `BypassDirect` | **0 GB** |
| `scontent*.fbcdn.net`, `*.gvt1.com`, `fonts.gstatic.com`, trackers/ads | **`REJECT`** (204 / Harmless JS Stub) | `Block204` / `MockScript` | **0 GB** |

**Dynamic Split Routing in Proxy Guard (v1.14.0+):**
Add custom `DIRECT` bypass domains dynamically via REST API or MCP (`proxy-guard__configure_rules`):
```bash
curl -X POST http://127.0.0.1:3128/custom/proxy-guard/rules \
  -H "Content-Type: application/json" \
  -d '{
    "add_direct_bypass_domains": ["cdn.jsdelivr.net", "cdnjs.cloudflare.com", "unpkg.com"]
  }'
```

**Open-Source Local Split-Router Alternatives (Standalone / Multi-Upstream):**
- **`MetaCubeX/mihomo`** *(Clash Meta core)*: Flexible `DOMAIN` / `DOMAIN-SUFFIX` rule engine (e.g., `services.sheerid.com` → DataImpulse proxy, `muse.ai` → Static US ISP, heavy CDNs/trackers → `DIRECT` or `REJECT`).
- **`apify/proxy-chain`** *(Node.js / TypeScript)*: Programmable local proxy server for scraping/automation with upstream HTTP/HTTPS/SOCKS5 chaining and dynamic per-request routing/blocking.
- **`go-gost/gost`** *(Go)*: Lightweight tunnel/proxy router with rule-based routing and basic forward/reverse proxy caching.

---

#### Group 2: HTTP Client-Layer Caching (Pure REST API Tools)

For tools calling REST APIs directly (such as repeated institution/school search endpoints or static configuration metadata), cache responses directly inside the HTTP client:
- **`karpetrosyan/hishel`** *(Python)*: Modern RFC 9111 HTTP caching for `httpx` and `requests` (both sync and async) backed by SQLite or Redis.
- **`requests-cache/requests-cache`** *(Python)*: Drop-in `CachedSession()` replacement for `requests.Session()` with automatic SQLite/Redis/file persistence and TTL / `Cache-Control` reuse.

---

#### Group 3: MITM Caching Proxy (Static/CDN Hosts Only)

When a language-agnostic caching proxy is needed in front of an upstream proxy to cache static JS, CSS, and JSON responses:
- **`investigativedata/mitmproxy-anycache`**: `mitmproxy` addon (with Docker image) caching responses by MIME type to file or Redis backends.
- **`kitsuyui/python-mitmcache`**: Lightweight `mitmproxy` addon for local HTTP/HTTPS response caching.

> [!CAUTION]
> **TLS Fingerprint Warning (`mitmproxy --mode upstream`):**
> Decrypting HTTPS via MITM terminates the browser's TLS connection and opens a new TLS handshake using `mitmproxy`'s OpenSSL stack. Strict anti-bot systems such as **`services.sheerid.com`** and **`muse.ai`** inspect JA3/JA4 TLS fingerprints and may flag or block MITM traffic.
> - **Always exclude sensitive hosts** with `--ignore-hosts '^(services\.sheerid\.com|muse\.ai|.*\.muse\.ai|auth\.meta\.com):443$'`.
> - For browser automation, prefer **Group 1 (non-MITM local proxy split routing)** combined with a **persistent browser disk cache** (see [Playwright Integration Guide](./PLAYWRIGHT_INTEGRATION.md#40-recommended-alternative-persistent-disk-cache--local-proxy-split-routing)).

---

## 4. Automation Tool Integration Examples

> 💡 **Dedicated Playwright Guide**: For in-depth recipes including in-browser route mocking, Python async flows, CI test runner configuration, and dynamic proxy rotation, see the dedicated [**Playwright Integration Guide**](./PLAYWRIGHT_INTEGRATION.md).

### 4.1 Playwright (TypeScript / Node.js)

Configure Playwright to route all browser traffic through Proxy Guard on `http://127.0.0.1:3128`:

```typescript
import { chromium } from 'playwright';

async function run() {
  const browser = await chromium.launch({
    headless: true,
    proxy: {
      server: 'http://127.0.0.1:3128',
    },
    args: [
      // Recommended flags to disable internal browser noise at launch
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-default-apps',
      '--no-default-browser-check',
      '--disable-sync',
      '--disable-features=OptimizationHints,TranslateUI,MediaRouter',
    ],
  });

  const context = await browser.newContext({
    // Optional: Pass session and per-request strategy headers
    extraHTTPHeaders: {
      'X-Proxy-Strategy': 'lowest_failures', // or 'sticky_session', 'round_robin', 'random'
      'X-Proxy-Session-Id': 'session-worker-42',
      'X-Proxy-Country': 'US',
    },
  });

  const page = await context.newPage();
  await page.goto('https://content.sheerid.com/verify');
  
  console.log('Page loaded with zero wasted bandwidth!');
  await browser.close();
}

run();
```

### 4.2 Playwright (Python)

```python
from playwright.sync_api import sync_playwright

with sync_playwright() as p:
    browser = p.chromium.launch(
        headless=True,
        proxy={"server": "http://127.0.0.1:3128"},
        args=[
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--no-default-browser-check",
            "--disable-sync",
        ],
    )
    context = browser.new_context(
        extra_http_headers={"X-Proxy-Session-Id": "profile-01"}
    )
    page = context.new_page()
    page.goto("https://quotes.toscrape.com")
    print(page.title())
    browser.close()
```

### 4.3 Puppeteer (Node.js)

```javascript
const puppeteer = require('puppeteer');

(async () => {
  const browser = await puppeteer.launch({
    headless: 'new',
    args: [
      '--proxy-server=http://127.0.0.1:3128',
      '--disable-background-networking',
      '--disable-component-update',
    ],
  });

  const page = await browser.newPage();
  await page.goto('https://example.com');
  await browser.close();
})();
```

### 4.4 cURL / Direct Scraping (Python Requests)

```python
import requests

proxies = {
    "http": "http://127.0.0.1:3128",
    "https": "http://127.0.0.1:3128",
}

response = requests.get(
    "https://quotes.toscrape.com",
    proxies=proxies,
    headers={"X-Proxy-Session-Id": "scraper-001"}
)
print("Status:", response.status_code)
```

---

## 5. Model Context Protocol (MCP) Tools for AI Agents

When interacting with AI agents (e.g. Gemini, Antigravity, Claude), Proxy Guard registers the following declarative MCP tools under the `proxy-guard__` namespace:

| Tool Name | Purpose | Key Arguments |
| :--- | :--- | :--- |
| `proxy-guard__evaluate_url` | Evaluates a single URL and reports whether it will be mocked, blocked, or forwarded. | `url`, `method`, `resource_type` |
| `proxy-guard__optimize_flow` | Analyzes a complete batch of HTTP requests or HAR 1.2 log entries and computes exact MB and dollar savings. | `requests` (Array of objects or HAR entries) |
| `proxy-guard__add_upstream_proxy` | Dynamically registers a paid upstream proxy into the rotation pool. | `url`, `name`, `country`, `tags` |
| `proxy-guard__list_upstream_proxies` | Lists all registered upstream proxies with credentials securely masked. | *(none)* |
| `proxy-guard__remove_upstream_proxy` | Unregisters a proxy from the pool by ID or URL. | `id` |
| `proxy-guard__set_rotation_strategy` | Updates rotation strategy (`round_robin`, `sticky_session`, etc.). | `strategy` |
| `proxy-guard__get_next_proxy` | Selects the next healthy upstream proxy according to strategy and filters. | `country`, `session_id`, `strategy` |
| `proxy-guard__get_metrics` | Returns cumulative metrics on bytes saved, blocked requests, and USD saved. | *(none)* |
| `proxy-guard__get_request_logs` | Retrieves live ring-buffer logs of intercepted requests and cumulative request reduction percentage. | `limit` (optional integer, default 50) |
| `proxy-guard__configure_rules` | Toggles rule categories or configures custom domain whitelists. | `block_analytics`, `block_ads`, `whitelist` |
| `proxy-guard__generate_script` | Outputs ready-to-use interceptor code for Playwright, Puppeteer, or mitmproxy. | `target` (`playwright_ts`, `playwright_py`, etc.) |

---

## 6. Running and Testing the Module

### 6.1 Starting the Proxy Guard Listener

Execute `run.sh` with `--proxy-port`:

```bash
# Start on port 3128 and forward allowed traffic to an upstream proxy
./modules/proxy-guard/run.sh --proxy-port 3128 --upstream "http://user:pass@pr.oxylabs.io:7777"
```

### 6.2 Verifying Interception

1. **Verify Analytics Mocking**:
   ```bash
   curl -i -x http://127.0.0.1:3128 http://www.google-analytics.com/analytics.js
   ```
   *Expected Response: HTTP 200 OK with `window.dataLayer = window.dataLayer || [];` and 0 bytes forwarded upstream.*

2. **Verify Browser Bloat Blocking**:
   ```bash
   curl -i -x http://127.0.0.1:3128 http://edgedl.me.gvt1.com/update
   ```
   *Expected Response: HTTP 204 No Content.*

3. **Verify Upstream Forwarding for Core Sites**:
   ```bash
   curl -i -x http://127.0.0.1:3128 https://quotes.toscrape.com
   ```
   *Expected Response: HTTP 200 OK forwarded through the selected healthy proxy in the pool.*

4. **Verify Real-Time Metrics & Reduction Logs**:
   ```bash
   # Cumulative counters
   curl http://127.0.0.1:3128/custom/proxy-guard/metrics

   # Live intercepted request logs & reduction percentage
   curl -s "http://127.0.0.1:3128/custom/proxy-guard/logs?limit=10"

   # Clear logs buffer
   curl -s -X POST http://127.0.0.1:3128/custom/proxy-guard/logs/clear
   ```
