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
    PG_Rules -->|Blocked / Mocked| PW_Browser
    PG_Rules -->|Forwarded| PG_Pool
    PG_Pool --> Upstream --> Target

    PW_Context -->|"Mode 2: context.route('**/*') abort/fulfill"| PW_Route
    PW_Route -->|204 No Content / Mock Stub| PW_Context
```

- **Mode 1: Native Proxy Forwarding (`127.0.0.1:3128`)**: Zero code changes. Playwright points its proxy server to Proxy Guard. Proxy Guard strips headers, blocks heavy media, mocks analytics, and handles upstream proxy failover automatically.
- **Mode 2: In-Browser Route Interception (`page.route()`)**: Blocks or mocks requests inside the browser process before a network socket is even opened. Ideal when you cannot run a local proxy process.
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

## 4. Mode 2: In-Browser Route Interception (`page.route()`)

If you run Playwright in environments where spawning a background proxy daemon is restricted (e.g. AWS Lambda, Google Cloud Run), you can use Proxy Guard's **Client Script Generator** to intercept and mock bloat directly inside Playwright.

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

async function getNextProxy(country = 'US', sessionId = 'session-1', strategy = 'lowest_failures'): Promise<PlaywrightProxyConfig> {
  const response = await fetch(
    `http://127.0.0.1:28130/custom/proxy-guard/proxies/next?country=${country}&session_id=${sessionId}&strategy=${strategy}`
  );
  const data = await response.json();
  if (data.status !== 'success' || !data.playwright_config) {
    throw new Error('No available proxy returned from Proxy Guard pool');
  }
  return data.playwright_config;
}

async function runSession() {
  // Retrieve balanced proxy config from Proxy Guard pool
  const proxyConfig = await getNextProxy('US', 'account-user-123');

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

3. **Check Live Dashboard**:
   Open `http://127.0.0.1:28130/custom/proxy-guard/ui` or `/modules/proxy-guard/config` to view:
   - Total requests processed vs. blocked
   - Megabytes transferred vs. saved
   - Estimated cost savings in USD
   - Active upstream multi-proxy pool health status
