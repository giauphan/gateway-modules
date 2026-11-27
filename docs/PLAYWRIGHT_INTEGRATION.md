# Playwright Integration Guide for Proxy Guard & Toolcase Gateway

Comprehensive guide for integrating **Playwright** (TypeScript, Node.js, and Python) with **Proxy Guard** to reduce paid proxy bandwidth usage by **60% – 95%**, enforce sticky IP rotation, and preserve anti-bot verification (Cloudflare Turnstile, SheerID).

---

## 1. Why Integrate Playwright with Proxy Guard?

When running automated browser workflows through paid residential, mobile, or datacenter proxies ($2 – $15 / GB), headless Chromium downloads enormous amounts of hidden background bloat on every new session:

| Resource Type | Real-World Traffic | Proxy Guard Action | Bandwidth Saved | Impact on Automation |
| :--- | :--- | :--- | :---: | :--- |
| **Browser Background Bloat** | `edgedl.me.gvt1.com`, `ciscobinary.openh264.org`, `update.googleapis.com` | **Blocked (204 No Content)** | **~9.67 MB** | **0% impact**: Internal Chromium update blobs, websites never invoke them. |
| **Heavy Video & Audio** | `*.mp4`, `*.webm`, `*.m3u8`, `*.ts`, `*.mp3` | **Blocked (204 No Content)** | **~5.00 MB – 20 MB** | **0% impact**: Scrapers extracting text, prices, or forms do not need video rendering. |
| **Web Fonts** | `fonts.gstatic.com`, `fonts.googleapis.com` | **Blocked (204 No Content)** | **~0.15 MB** | **0% impact**: Browser cleanly falls back to system fonts (Arial, sans-serif); DOM selectors & clicks are unchanged. |
| **Analytics & Tag Managers** | `googletagmanager.com`, `google-analytics.com` | **Synthetic JS Mock (200 OK)** | **~0.20 MB** | **0% impact**: Provides harmless `window.dataLayer`, `gtag()`, `ga()` stubs so client scripts do not throw runtime exceptions. |
| **Pixels & Error Telemetry** | `connect.facebook.net`, `sentry.io`, `newrelic.com` | **Synthetic JS Mock / Empty JSON** | **~0.08 MB** | **0% impact**: Provides `window.fbq()`, `Sentry.captureException()` stubs. |
| **Anti-Fraud & Verification** | `online-metrix.net` (ThreatMetrix), `sheerid.com` | **Forwarded (Allowed)** | 0 KB | **Preserved**: Required by sensitive identity and verification platforms. |
| **Cloudflare Turnstile** | `challenges.cloudflare.com` | **Forwarded (Allowed)** | 0 KB | **Preserved**: Allows bot / automation tools to render and solve Turnstile Captchas cleanly. |

---

## 2. Architecture & Integration Modes

Proxy Guard supports three integration patterns with Playwright depending on your architecture:

```mermaid
flowchart TD
    subgraph Playwright ["Playwright Automation Engine"]
        PW_Browser["Chromium / Firefox / WebKit"]
        PW_Context["BrowserContext / Page"]
    end

    subgraph ModeA ["Mode 1: Native HTTP/CONNECT Proxy (Port 3128)"]
        PG_Server["Proxy Guard Interceptor (127.0.0.1:3128)"]
        PG_Rules["Rules Engine & Request Optimizer"]
        PG_Pool["Multi-Proxy Pool & Smart Rotator"]
    end

    subgraph ModeB ["Mode 2: In-Browser Route Mocking (page.route)"]
        PW_Route["playwright_ts / playwright_py Script Interceptor"]
    end

    subgraph External ["Upstream Proxy Networks & Target Websites"]
        Upstream["Residential / Mobile Proxy (Oxylabs, BrightData, Smartproxy)"]
        Target["Target Website (e-Commerce, Travel, SheerID)"]
    end

    PW_Browser -->|"Mode 1: proxy: { server: 'http://127.0.0.1:3128' }"| PG_Server
    PG_Server --> PG_Rules
    PG_Rules -->|Blocked / Mocked (0 GB)| PW_Browser
    PG_Rules -->|BypassDirect / DIRECT (0 GB paid proxy)| Target
    PG_Rules -->|Forwarded (Sensitive Hosts)| PG_Pool
    PG_Pool --> Upstream --> Target

    PW_Context -->|"Mode 2: context.route('**/*') abort/fulfill (disables HTTP cache!)"| PW_Route
    PW_Route -->|204 No Content / Mock Stub| PW_Context
```

- **Mode 1: Native Proxy Forwarding & Domain Split Routing (`127.0.0.1:3128`) *(Recommended)***: Zero code changes. Playwright points its proxy server to Proxy Guard. Proxy Guard blocks heavy bloat (`REJECT`), routes public static CDNs `DIRECT` (`BypassDirect`), and forwards only sensitive domains (`services.sheerid.com`, `muse.ai`, `auth.meta.com`) to the upstream proxy pool — **preserving both the browser's HTTP cache AND original TLS fingerprint**.
- **Mode 2: In-Browser Route Interception (`page.route()`) *(Fallback Only)***: Blocks or mocks requests inside the browser process when you cannot run a local proxy. **Note:** Enabling `page.route()` disables Chromium's HTTP cache (see [Section 4](#4-mode-2-in-browser-route-interception-pageroute--http-cache-trap)).
- **Mode 3: Dynamic Multi-Proxy Rotation**: Query Proxy Guard's `/proxies/next` endpoint to receive a Playwright-ready proxy dictionary (`{ server, username, password }`).

---

## 3. Mode 1: Native Proxy Forwarding (Recommended)

### 3.1 Launch Arguments & Configuration
Always pass recommended Chromium flags to prevent background update downloads before navigation begins:

```text
--disable-background-networking
--disable-component-update
--disable-default-apps
--no-default-browser-check
--disable-sync
--disable-features=OptimizationHints,TranslateUI,MediaRouter
```

### 3.2 TypeScript / Node.js Implementation

```typescript
import { chromium, Browser, BrowserContext, Page } from 'playwright';

async function main() {
  const browser: Browser = await chromium.launch({
    headless: true,
    proxy: {
      server: 'http://127.0.0.1:3128',
    },
    args: [
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-default-apps',
      '--no-default-browser-check',
      '--disable-sync',
      '--disable-features=OptimizationHints,TranslateUI,MediaRouter',
    ],
  });

  // Create isolated context with session headers for sticky rotation
  const context: BrowserContext = await browser.newContext({
    viewport: { width: 1920, height: 1080 },
    userAgent:
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
    extraHTTPHeaders: {
      // Per-request rotation strategy: round_robin, sticky_session, lowest_failures, or random
      'X-Proxy-Strategy': 'lowest_failures',
      // Pin session to a consistent upstream IP in Proxy Guard's multi-proxy pool
      'X-Proxy-Session-Id': 'worker-profile-42',
      // Optional: Filter proxy pool by ISO country code
      'X-Proxy-Country': 'US',
    },
  });

  const page: Page = await context.newPage();
  
  console.log('Navigating to target with Proxy Guard active...');
  await page.goto('https://quotes.toscrape.com', { waitUntil: 'domcontentloaded' });
  
  console.log('Title:', await page.title());
  
  await context.close();
  await browser.close();
}

main().catch(console.error);
```

### 3.3 Python Implementation (Sync & Async)

#### Synchronous Python (`sync_playwright`)
```python
from playwright.sync_api import sync_playwright

def scrape_with_proxy_guard():
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
            extra_http_headers={
                "X-Proxy-Strategy": "lowest_failures",
                "X-Proxy-Session-Id": "python-worker-01",
                "X-Proxy-Country": "US",
            }
        )
        page = context.new_page()

        page.goto("https://quotes.toscrape.com", wait_until="domcontentloaded")
        print(f"Loaded page title: {page.title()}")

        context.close()
        browser.close()

if __name__ == "__main__":
    scrape_with_proxy_guard()
```

#### Asynchronous Python (`async_playwright`)
```python
import asyncio
from playwright.async_api import async_playwright

async def async_scrape():
    async with async_playwright() as p:
        browser = await p.chromium.launch(
            headless=True,
            proxy={"server": "http://127.0.0.1:3128"},
            args=["--disable-background-networking", "--disable-component-update"],
        )
        context = await browser.new_context(
            extra_http_headers={"X-Proxy-Session-Id": "async-worker-99"}
        )
        page = await context.new_page()

        await page.goto("https://quotes.toscrape.com")
        print(f"Async title: {await page.title()}")

        await context.close()
        await browser.close()

if __name__ == "__main__":
    asyncio.run(async_scrape())
```

---

## 4. Mode 2: In-Browser Route Interception (`page.route()`) & HTTP Cache Trap

> [!WARNING]
> **Critical Playwright / Puppeteer Trap (`page.route()` Disables HTTP Cache):**
> Enabling request interception (`page.route()` / `context.route()` in Playwright, or `page.setRequestInterception(true)` in Puppeteer) **automatically disables the browser's built-in HTTP cache**. As a result, heavy JavaScript bundles (often 2–8 MB each) are re-downloaded on every page load or session instead of being served from disk cache.

### 4.0 Recommended Alternative: Persistent Disk Cache + Local Proxy Split Routing

To achieve maximum GB savings **without** triggering the `page.route()` cache trap or breaking TLS fingerprints on strict platforms (`services.sheerid.com`, `muse.ai`):

1. **Preserve Browser Disk Cache Across Sessions (`launchPersistentContext` / `user_data_dir`)**:
   Instead of launching an ephemeral blank profile every run, reuse a persistent `user_data_dir` (or shared cache directory) so static JS/CSS assets are served from local disk cache at **0 bytes**:

   **TypeScript (`launchPersistentContext` + Proxy Guard Split Routing + Sticky Session + WebRTC Protection):**
   ```typescript
   import { chromium } from 'playwright';

   const context = await chromium.launchPersistentContext('./cache/pw-profile', {
     headless: true,
     timezoneId: 'America/New_York',
     locale: 'en-US',
     // Block ads/fonts/media & split-route static CDNs DIRECT at the local proxy layer
     // Pass `<pool>:<session_id>` via username to pin a sticky residential IP!
     // (NO page.route() needed — HTTP disk cache and TLS fingerprint stay 100% intact!)
     proxy: {
       server: 'http://127.0.0.1:3128',
       username: 'residential:sheerid-session-01',
       password: 'x',
     },
     args: [
       '--disable-background-networking',
       '--disable-component-update',
       '--disable-default-apps',
       '--no-default-browser-check',
       '--disable-sync',
       '--force-webrtc-ip-handling-policy=disable_non_proxied_udp',
       '--enforce-webrtc-ip-permission-check',
     ],
   });
   ```

   **Python (`launch_persistent_context` + Proxy Guard Split Routing + Sticky Session + WebRTC Protection):**
   ```python
   from playwright.sync_api import sync_playwright

   with sync_playwright() as p:
       context = p.chromium.launch_persistent_context(
           user_data_dir="./cache/pw-profile",
           headless=True,
           timezone_id="America/New_York",
           locale="en-US",
           proxy={
               "server": "http://127.0.0.1:3128",
               "username": "residential:sheerid-session-01",
               "password": "x",
           },
           args=[
               "--disable-background-networking",
               "--disable-component-update",
               "--disable-default-apps",
               "--no-default-browser-check",
               "--disable-sync",
               "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
               "--enforce-webrtc-ip-permission-check",
           ],
       )
       page = context.new_page()
       page.goto("https://services.sheerid.com", wait_until="domcontentloaded")
       context.close()
   ```

2. **Block Junk & Split-Route at the Local Proxy Layer**:
   Use **Proxy Guard (`127.0.0.1:3128`)**, **`MetaCubeX/mihomo`**, or **`apify/proxy-chain`** to reject ads/fonts/trackers (`REJECT`) and bypass public CDNs (`DIRECT`) at the network layer instead of `page.route()`. This saves **70% – 95% paid proxy GB** while preserving both Chromium's HTTP cache and the browser's genuine TLS fingerprint.

---

If you run Playwright in environments where spawning a background proxy daemon is strictly impossible (e.g. AWS Lambda, Google Cloud Run), you can fall back to Proxy Guard's **Client Script Generator** below:

### 4.1 Generate Script Dynamically via Gateway / MCP
Call Proxy Guard's API or MCP tool `proxy-guard__generate_script` with target `playwright_ts` or `playwright_py`:

```bash
curl -s http://127.0.0.1:28130/custom/proxy-guard/scripts/playwright_ts
```

### 4.2 TypeScript / Node.js Interceptor Snippet

```typescript
import { BrowserContext, Route } from 'playwright';

export async function attachProxyGuardInterceptor(context: BrowserContext): Promise<void> {
  const TRANSPARENT_1X1_GIF = Buffer.from(
    'R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',
    'base64'
  );

  const GA_GTM_STUB = `
    window.dataLayer = window.dataLayer || [];
    window.gtag = window.gtag || function() { window.dataLayer.push(arguments); };
    window.ga = window.ga || function(cb) { if (typeof cb === 'function') cb(); };
    window.google_tag_manager = window.google_tag_manager || {};
  `;

  const FB_PIXEL_STUB = `
    window.fbq = window.fbq || function() {};
    window._fbq = window.fbq;
  `;

  await context.route('**/*', async (route: Route) => {
    const request = route.request();
    const url = request.url().toLowerCase();
    const resourceType = request.resourceType();

    // 1. CRITICAL: Never block Anti-Bot / CAPTCHAs or Identity Verification
    if (
      url.includes('challenges.cloudflare.com') ||
      url.includes('online-metrix.net') ||
      url.includes('sheerid.com')
    ) {
      return route.continue();
    }

    // 2. Mock Analytics & Tag Managers (prevents script errors)
    if (url.includes('googletagmanager.com') || url.includes('google-analytics.com')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/javascript',
        body: GA_GTM_STUB,
      });
    }

    // 3. Mock Tracking Pixels
    if (url.includes('connect.facebook.net') || url.includes('fbq')) {
      return route.fulfill({
        status: 200,
        contentType: 'application/javascript',
        body: FB_PIXEL_STUB,
      });
    }

    // 4. Block Heavy Media & Fonts
    if (resourceType === 'image' && (url.includes('/pixel') || url.includes('collect'))) {
      return route.fulfill({
        status: 200,
        contentType: 'image/gif',
        body: TRANSPARENT_1X1_GIF,
      });
    }

    if (
      resourceType === 'media' ||
      resourceType === 'font' ||
      url.endsWith('.mp4') ||
      url.endsWith('.webm') ||
      url.endsWith('.woff2') ||
      url.includes('fonts.gstatic.com')
    ) {
      return route.abort('blockedbyclient');
    }

    // 5. Allow essential documents, APIs, and scripts
    return route.continue();
  });
}
```

---

## 5. Mode 3: Dynamic Multi-Proxy Rotation via API / MCP

When your Playwright script needs to rotate residential proxies per worker or account:

```typescript
import { chromium } from 'playwright';

interface PlaywrightProxyConfig {
  server: string;
  username?: string;
  password?: string;
}

// Fallback configuration when upstream pool in Proxy Guard is empty or uninitialized
const LOCAL_FALLBACK_PROXY: PlaywrightProxyConfig = {
  server: process.env.FALLBACK_PROXY_SERVER || 'http://pr.oxylabs.io:7777',
  username: process.env.FALLBACK_PROXY_USERNAME,
  password: process.env.FALLBACK_PROXY_PASSWORD,
};

async function getNextProxy(country = 'US', sessionId = 'session-1', strategy = 'lowest_failures'): Promise<PlaywrightProxyConfig> {
  try {
    // Both POST with JSON body and GET with query params are supported
    const response = await fetch('http://127.0.0.1:28130/custom/proxy-guard/proxies/next', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ country, session_id: sessionId, strategy }),
    });

    if (!response.ok) {
      // Handles HTTP 404: {"error":"No healthy upstream proxy available in pool"}
      const err = await response.json().catch(() => ({ error: response.statusText }));
      console.warn(`[ProxyGuard] Upstream pool warning (${response.status}): ${err.error}. Falling back to default proxy.`);
      return LOCAL_FALLBACK_PROXY;
    }

    const data = await response.json();
    if (data.status === 'success' && data.playwright_config) {
      return data.playwright_config;
    }
  } catch (err) {
    console.warn('[ProxyGuard] Gateway unreachable, using fallback proxy:', err);
  }
  return LOCAL_FALLBACK_PROXY;
}

async function runSession() {
  // Retrieve balanced proxy config from Proxy Guard pool (or fallback)
  const proxyConfig = await getNextProxy('US', 'account-user-123', 'lowest_failures');

  const browser = await chromium.launch({
    headless: true,
    proxy: proxyConfig,
  });

  const page = await browser.newPage();
  await page.goto('https://api.ipify.org?format=json');
  console.log('Assigned IP:', await page.textContent('body'));

  await browser.close();
}
```

---

## 6. Playwright Test Runner (`playwright.config.ts`)

For test suites running on CI/CD pipelines, configure Proxy Guard globally in `playwright.config.ts`:

```typescript
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  use: {
    /* Route all network requests through Proxy Guard */
    proxy: {
      server: 'http://127.0.0.1:3128',
    },
    extraHTTPHeaders: {
      'X-Proxy-Session-Id': 'ci-test-suite',
    },
    launchOptions: {
      args: [
        '--disable-background-networking',
        '--disable-component-update',
        '--disable-default-apps',
      ],
    },
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
```

---

## 7. Real-World Bandwidth Benchmark

Comparison of bandwidth consumed by Playwright navigating real-world target flows with and without Proxy Guard:

```
+------------------------------------+------------------+---------------------+-------------------+
| Target Platform Flow               | Direct Upstream  | With Proxy Guard    | Bandwidth Saved   |
+------------------------------------+------------------+---------------------+-------------------+
| SheerID Student Verification       | 15.68 MB         | 0.82 MB             | 94.7% ($ saved)   |
| High-Traffic E-Commerce Search     | 24.30 MB         | 1.45 MB             | 94.0% ($ saved)   |
| Cloudflare Turnstile Challenge Page|  6.80 MB         | 5.10 MB (Protected) | 25.0% (Turnstile) |
| Travel / Hotel Booking Aggregator  | 32.10 MB         | 2.80 MB             | 91.3% ($ saved)   |
| Quote / Content Scraping Catalog   |  0.42 MB         | 0.05 MB             | 88.1% ($ saved)   |
+------------------------------------+------------------+---------------------+-------------------+
```

---

## 8. Verification & Live Monitoring

1. **Verify Connectivity**:
   ```bash
   curl -x http://127.0.0.1:3128 https://api.ipify.org
   ```

2. **Verify Header Stripping & Compression**:
   Run your Playwright script and monitor Proxy Guard logs:
   ```bash
   # Terminal / Journal logs
   [proxy-guard] Local intercepting proxy listening on 127.0.0.1:3128
   ```

3. **Inspect Live Request Audit Logs & Reduction % (v1.5.0+)**:
   While Playwright tests run, monitor exactly which background bloat and telemetry requests are blocked vs. forwarded upstream:
   ```bash
   # Query recent 20 intercepted requests & reduction rate
   curl -s "http://127.0.0.1:28130/custom/proxy-guard/logs?limit=20"
   ```
   Sample output:
   ```json
   {
     "status": "success",
     "request_reduction_percentage": 82.4,
     "blocked_requests": 140,
     "forwarded_requests": 30,
     "total_requests": 170,
     "logs": [
       {
         "id": 170,
         "timestamp": 1700923200,
         "method": "GET",
         "host": "ciscobinary.openh264.org",
         "path": "/openh264-win64-2e17de1.zip",
         "action": "Blocked (204 No Content)",
         "category": "Browser Bloat (Codecs)",
         "bytes_saved": 535552,
         "is_blocked": true
       }
     ]
   }
   ```
   To reset the ring buffer during test runs:
   ```bash
   curl -s -X POST http://127.0.0.1:28130/custom/proxy-guard/logs/clear
   ```

4. **MCP Tool for AI Agent Verification**:
   AI agents can inspect intercepted request logs dynamically by invoking:
   ```json
   {
     "name": "proxy-guard__get_request_logs",
     "arguments": {
       "limit": 25
     }
   }
   ```

5. **Check Interactive Web Dashboard**:
   Open `http://127.0.0.1:28130/custom/proxy-guard/ui` or `/modules/proxy-guard/config` to view:
   - **Request Reduction % Card** (real-time efficiency metric)
   - Total requests processed vs. blocked
   - Megabytes transferred vs. saved
   - Estimated cost savings in USD
   - Live Intercepted Request Logs table with status badges (`Blocked`, `Mocked`, `Forwarded`)
   - Active upstream multi-proxy pool health status
