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
│  1. Request Classifier & Rules Engine                       │
│     ├── Block & Mock (0 Bytes sent upstream):               │
│     │   ├── Browser Bloat (*.gvt1.com Widevine: ~9.1MB)     │
│     │   ├── OpenH264 Video Codec (~523KB)                   │
│     │   ├── Google Tag Manager / GA4 (Mock JS Stub)         │
│     │   ├── Meta Pixel, Sentry, New Relic (Mock JS Stub)    │
│     │   └── Custom Web Fonts (.woff2, .ttf)                 │
│     └── Allowed Core Traffic:                               │
│         ├── Request Optimizer (strip utm_*, fbclid, sec-ch) │
│         └── RFC 9110 Safe Compression Rewriter             │
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

#### Per-Request Strategy Override (v1.4.0+)
Instead of changing the global strategy for all workers, individual requests or browser contexts can override the rotation strategy on the fly:

1. **Via HTTP Header (`X-Proxy-Strategy`)**:
   Pass `X-Proxy-Strategy: lowest_failures` (or `round_robin`, `sticky_session`, `random`) in your request or Playwright `extraHTTPHeaders`. Proxy Guard routes the request using that strategy and automatically strips the header before forwarding upstream.

2. **Via REST API (`/proxies/next`)**:
   Query parameters or request body take immediate precedence:
   ```bash
   # Query parameter
   curl -s "http://127.0.0.1:3128/custom/proxy-guard/proxies/next?strategy=lowest_failures&country=US"

   # POST JSON Body
   curl -s -X POST http://127.0.0.1:3128/custom/proxy-guard/proxies/next \
     -H "Content-Type: application/json" \
     -d '{"strategy": "sticky_session", "session_id": "profile-99"}'
   ```

3. **Via MCP Tool**:
   Call `proxy-guard__get_next_proxy` with argument `"strategy": "lowest_failures"`.

---

### 2.4 Managing Proxies in the UI

You can manage and save multi-proxies in two places within the user interface:

#### Method A: Toolcase Gateway Settings UI (Configuration Page)
- **Path**: Navigate to **Modules** > **proxy-guard** > **Configuration** (or open `http://<host>:<port>/modules/proxy-guard/config`).
- **Field**: In the **"Upstream Multi-Proxy Pool (1 per line)"** textarea, paste multiple proxies (one per line). Supported formats include standard URLs with credentials (`http://user:pass@host:port`), SOCKS5 (`socks5://user:pass@host:port`), or shorthand vendor syntax (`host:port:user:pass`).
- **Rotation Strategy**: Set the default strategy in the **"Proxy Rotation Strategy"** input (`round_robin`, `sticky_session`, `lowest_failures`, `random`).
- **Save**: Click **Save Settings**. Proxy Guard saves the pool to disk (`data/proxies.json`) and instantly reloads the pool into memory.

#### Method B: Interactive Proxy Guard Dashboard
- **Path**: Open `http://<host>:<port>/custom/proxy-guard/ui` in your browser.
- **Features**:
  - Live metrics: Total active proxies, current rotation strategy, MB saved, and estimated cost savings in USD.
  - Proxy health table: Protocol, masked auth, host:port, country, tags, success/failure counts, and real-time status badges (`Healthy`, `Recovering`, `Failing`).
  - **➕ Add Multiple Upstream Proxies**: Enter proxies in the multi-line textarea with optional batch Country Code and Tags, then click **Add to Proxy Pool**.
  - One-click removal via the **Remove** button on any table row.

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
| `proxy-guard__get_metrics` | Returns cumulative metrics on bytes saved, blocked requests, and USD saved. | *(none)* |
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

4. **Verify Real-Time Metrics**:
   ```bash
   curl http://127.0.0.1:3128/custom/proxy-guard/metrics
   ```
