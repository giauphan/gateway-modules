# Proxy Guard & Multi-Proxy API Payload Reference

> **Comprehensive API Reference & Request/Response Payload Specifications for `proxy-guard` (v1.15.2).**  
> Covers REST endpoints, query parameters, JSON payloads, auto-detection rules, and MCP tool arguments.

---

## Table of Contents
1. [Endpoint Quick Reference](#1-endpoint-quick-reference)
2. [Proxy Selection: `POST` & `GET /proxies/next`](#2-proxy-selection-post--get-proxiesnext)
   - [Request Payload Schema](#21-request-payload-schema)
   - [Field Specifications & Behaviors](#22-field-specifications--behaviors)
   - [Automatic Country Selection & Fallback Rules](#23-automatic-country-selection--fallback-rules)
   - [Response Payload Schema](#24-response-payload-schema)
   - [Client Integration Examples](#25-client-integration-examples)
3. [Proxy Registration: `POST /proxies/add`](#3-proxy-registration-post-proxiesadd)
   - [Single Proxy Payload](#31-single-proxy-payload)
   - [Batch Proxies Payload](#32-batch-proxies-payload)
   - [Supported Proxy URL Formats](#33-supported-proxy-url-formats)
   - [Automatic Username Country Parsing](#34-automatic-username-country-parsing)
4. [Proxy Management Endpoints](#4-proxy-management-endpoints)
   - [`GET /proxies/list`](#41-get-proxieslist)
   - [`POST /proxies/remove`](#42-post-proxiesremove)
   - [`POST /proxies/strategy`](#43-post-proxiesstrategy)
5. [Traffic Optimization & Audit Logs](#5-traffic-optimization--audit-logs)
   - [`GET /logs` & Request Reduction %](#51-get-logs--request-reduction-)
   - [`POST /logs/clear`](#52-post-logsclear)
   - [`GET /metrics` & `POST /metrics/reset`](#53-get-metrics--post-metricsreset)
   - [`POST /evaluate`](#54-post-evaluate)
   - [`POST /optimize-flow`](#55-post-optimize-flow)
6. [MCP (Model Context Protocol) Tool Reference](#6-mcp-model-context-protocol-tool-reference)
7. [Error Codes & Troubleshooting Guide](#7-error-codes--troubleshooting-guide)

---

## 1. Endpoint Quick Reference

All endpoints are reachable under either direct paths (e.g. `http://127.0.0.1:3128/proxies/...`) or Gateway reverse-proxy mounted paths (e.g. `https://gateway.genrostore.com/custom/proxy-guard/proxies/...`).

| Method | Endpoint Path | Purpose | Request Body |
| :--- | :--- | :--- | :--- |
| **POST** / **GET** | `/proxies/next` | Select next upstream proxy with strategy & country filtering | JSON or Query Params |
| **POST** | `/proxies/add` | Register a single proxy or batch of proxies into pool | JSON (`url` or `proxies`) |
| **POST** | `/proxies/remove` | Unregister a proxy from the pool by ID | `{"id": "host:port"}` |
| **GET** | `/proxies/list` | List all registered proxies with credentials masked | *(none)* |
| **POST** | `/proxies/strategy` | Update global default rotation strategy | `{"strategy": "..."}` |
| **GET** | `/logs` | Get recent intercepted request audit trail & reduction % | Query `?limit=50` |
| **POST** | `/logs/clear` | Clear the in-memory request audit ring buffer | *(none)* |
| **GET** | `/metrics` | Get cumulative bandwidth and dollar savings counters | *(none)* |
| **POST** | `/metrics/reset`| Reset cumulative metrics counters | *(none)* |
| **POST** | `/evaluate` | Test if a single URL will be blocked, mocked, direct-bypassed, or forwarded | `{"url": "...", "method": "..."}` |
| **POST** | `/optimize-flow`| Analyze batch of requests or HAR log for bandwidth savings | `{"requests": [...]}` |
| **POST** | `/rules` | Toggle categories, add whitelist/blocked/direct-bypass domains, named `add_pool_routes`, `default_route`, or `cache` settings | `{"add_pool_routes": [...], "default_route": "direct"}` |
| **POST** | `/tools/call` | Model Context Protocol (MCP) tool execution | `{"name": "...", "arguments": {...}}` |

---

## 2. Proxy Selection: `POST` & `GET /proxies/next`

This is the primary endpoint used by automation runners, web crawlers, and headless browsers (Playwright, Puppeteer) to lease an upstream proxy configuration.

> **Recommendation**: Prefer `POST` with a JSON payload for robust transport across reverse proxies and Cloudflare tunnels. `GET` with query parameters is also fully supported.

### 2.1 Request Payload Schema

```json
{
  "country": "VN",
  "strategy": "lowest_failures",
  "session_id": "worker-profile-42",
  "tag": "residential",
  "preferred_id": "pr.oxylabs.io:7777"
}
```

#### Field Specifications:

| Field | Type | Required? | Default | Description & Allowed Values |
| :--- | :--- | :---: | :---: | :--- |
| `country` | `string` | Optional | `null` | Two-letter ISO 3166-1 alpha-2 country code (e.g. `"VN"`, `"US"`, `"DE"`, `"SG"`). **Case-insensitive**. Automatically filters proxies tagged or detected with this country. |
| `strategy` | `string` | Optional | Global pool strategy | Per-request balancing algorithm: <br>• `"round_robin"`: Equal distribution across candidates.<br>• `"sticky_session"`: Pin to single proxy per `session_id`.<br>• `"lowest_failures"`: Pick proxy with least recorded errors.<br>• `"random"`: Uniform random pick. |
| `session_id` | `string` | Optional | `null` | Persistent session key (e.g. user ID, profile name, or UUID). When using `"sticky_session"`, requests with the same `session_id` consistently receive the same upstream proxy until failure. |
| `tag` | `string` | Optional | `null` | Filter candidates by tag (e.g. `"residential"`, `"mobile"`, `"datacenter"`). |
| `preferred_id` | `string` | Optional | `null` | Specific proxy ID (usually `host:port`) to prefer if healthy. |

---

### 2.2 Field Specifications & Behaviors

#### `strategy` Options:
1. **`round_robin`**: Sequences requests across all eligible upstream proxies sequentially.
2. **`sticky_session`**: Hashes `session_id` and binds it to a dedicated proxy. If that proxy fails 3 consecutive times, it automatically migrates the session to the next healthy proxy.
3. **`lowest_failures`**: Selects candidate proxy with `min(failure_count)` and ties broken by `max(success_count)`. Ideal for high-stakes flows like account verification.
4. **`random`**: Cryptographically pseudo-random uniform selection across eligible candidates.

---

### 2.3 Automatic Country Selection & Fallback Rules

Proxy Guard uses a deterministic 3-tier country resolution workflow:

```text
Request Payload: {"country": "VN"}
         │
         ▼
1. Scan proxy pool for candidates with matching country ("VN" == "vn", case-insensitive)
         │
    ┌────┴────────────────────────┐
    │                             │
[Found >= 1 Matches]         [0 Matches Found]
    │                             │
    ▼                             ▼
Candidates = VN Proxies       Fallback: Candidates = All Healthy Proxies
    │                             │
    └──────────────┬──────────────┘
                   │
                   ▼
2. Apply rotation strategy (round_robin / lowest_failures / sticky) on Candidates
                   │
                   ▼
3. Format & return Playwright / Puppeteer credentials
```

1. **Exact Match Prioritization**: If the pool contains proxies tagged with `VN` (e.g. 5 Vietnam proxies and 3 US proxies), sending `{"country": "VN"}` **only** considers the 5 Vietnam proxies.
2. **Auto-Detection on Import**: If you add Oxylabs or BrightData proxies formatted with `-cc-vn`, Proxy Guard automatically infers `country: "VN"`.
3. **Graceful Fallback**: If the requested country is currently absent or all proxies for that country are unhealthy, Proxy Guard falls back to remaining healthy proxies in the pool so your automation run does not crash.

---

### 2.4 Response Payload Schema

#### HTTP 200 OK (Proxy Leased Successfully):
```json
{
  "status": "ok",
  "strategy_applied": "lowest_failures",
  "proxy": {
    "id": "pr.oxylabs.io:7777",
    "name": "pr.oxylabs.io:7777",
    "protocol": "http",
    "host": "pr.oxylabs.io",
    "port": 7777,
    "auth": {
      "username": "customer-resi_e58ec6d4-cc-vn",
      "password": "secret_password_here"
    },
    "country": "VN",
    "tags": ["residential"],
    "weight": 1,
    "enabled": true,
    "success_count": 42,
    "failure_count": 0,
    "url": "http://customer-resi_e58ec6d4-cc-vn:secret_password_here@pr.oxylabs.io:7777"
  },
  "playwright_config": {
    "server": "http://pr.oxylabs.io:7777",
    "username": "customer-resi_e58ec6d4-cc-vn",
    "password": "secret_password_here"
  },
  "puppeteer_args": [
    "--proxy-server=http://pr.oxylabs.io:7777"
  ]
}
```

#### Field Descriptions:
- **`status`**: `"ok"` when selection succeeds.
- **`strategy_applied`**: The exact strategy evaluated for this lease.
- **`proxy`**: Full domain entity including health counters and resolved country.
- **`playwright_config`**: Drop-in configuration object directly usable in `playwright.chromium.launch({ proxy: data.playwright_config })`.
- **`puppeteer_args`**: Drop-in CLI arguments for Puppeteer's `--proxy-server` launch flag.

#### HTTP 404 Not Found (Pool Empty or Unhealthy):
```json
{
  "error": "No healthy upstream proxy available in pool"
}
```

---

### 2.5 Client Integration Examples

#### cURL
```bash
# POST with JSON body (Recommended)
curl -s -X POST https://gateway.genrostore.com/custom/proxy-guard/proxies/next \
  -H "Content-Type: application/json" \
  -d '{"country": "VN", "strategy": "lowest_failures", "session_id": "worker-1"}'

# GET with Query Parameters
curl -s "https://gateway.genrostore.com/custom/proxy-guard/proxies/next?country=VN&strategy=round_robin"
```

#### Python (Playwright & Requests)
```python
import requests
from playwright.sync_api import sync_playwright

def get_leased_proxy(country="VN", session_id="task-01"):
    url = "https://gateway.genrostore.com/custom/proxy-guard/proxies/next"
    payload = {
        "country": country,
        "strategy": "lowest_failures",
        "session_id": session_id,
    }
    resp = requests.post(url, json=payload, timeout=5)
    if resp.status_code == 200:
        return resp.json()["playwright_config"]
    # Fallback to local default proxy if gateway pool is empty
    return {"server": "http://pr.oxylabs.io:7777", "username": "user", "password": "pass"}

with sync_playwright() as p:
    proxy_config = get_leased_proxy("VN", "session-worker-99")
    browser = p.chromium.launch(headless=True, proxy=proxy_config)
    page = browser.new_page()
    page.goto("https://api.ipify.org?format=json")
    print(page.inner_text("pre"))
    browser.close()
```

#### TypeScript / Node.js (Playwright)
```typescript
import { chromium } from 'playwright';

async function fetchProxy(country = 'VN') {
  const res = await fetch('https://gateway.genrostore.com/custom/proxy-guard/proxies/next', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ country, strategy: 'lowest_failures' }),
  });
  if (res.ok) {
    const data = await res.json();
    return data.playwright_config;
  }
  return null;
}

(async () => {
  const proxy = await fetchProxy('VN');
  const browser = await chromium.launch({ headless: true, proxy: proxy || undefined });
  const page = await browser.newPage();
  await page.goto('https://example.com');
  await browser.close();
})();
```

---

## 3. Proxy Registration: `POST /proxies/add`

Registers one or more upstream proxies into the pool. Changes are immediately persisted to `data/proxies.json`.

### 3.1 Single Proxy Payload

```json
{
  "url": "http://customer-resi_e58ec6d4-cc-vn:pass_with_equals=123@pr.oxylabs.io:7777",
  "name": "oxylabs-vn-residential",
  "country": "VN",
  "tags": ["residential", "premium"]
}
```

| Field | Type | Required? | Description |
| :--- | :--- | :---: | :--- |
| `url` | `string` | **Yes** | Upstream proxy URL. Supports standard formats, SOCKS5, and vendor shorthand. |
| `name` | `string` | Optional | Friendly identifier (defaults to `host:port`). |
| `country` | `string` | Optional | ISO alpha-2 country code. If omitted, Proxy Guard inspects username for `-cc-xx`. |
| `tags` | `array[string]`| Optional | Custom categorizations (e.g. `["residential", "mobile"]`). |

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "message": "Proxy registered successfully",
  "id": "pr.oxylabs.io:7777"
}
```

---

### 3.2 Batch Proxies Payload

Add an array of proxy objects or raw strings:

```json
{
  "proxies": [
    {
      "url": "http://customer-user-cc-vn:pass1@pr.oxylabs.io:7777",
      "country": "VN",
      "tags": ["vn-pool"]
    },
    {
      "url": "http://customer-user-cc-us:pass2@pr.oxylabs.io:7777",
      "country": "US",
      "tags": ["us-pool"]
    },
    "socks5://res-user:res-pass@10.0.0.5:1080"
  ]
}
```

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "added_count": 3,
  "added_ids": [
    "pr.oxylabs.io:7777",
    "pr.oxylabs.io:7777",
    "10.0.0.5:1080"
  ],
  "errors": []
}
```

---

### 3.3 Supported Proxy URL Formats

| Format Syntax | Example | Protocol |
| :--- | :--- | :--- |
| **Standard HTTP URL** | `http://user:pass@pr.oxylabs.io:7777` | HTTP |
| **SOCKS5 URL** | `socks5://user:pass@127.0.0.1:1080` | SOCKS5 (RFC 1928) |
| **Vendor Shorthand** | `pr.oxylabs.io:7777:user:pass` | HTTP |
| **Dual-Stack IPv6** | `http://user:pass@[2001:db8::1]:8080` | Dual-Stack IPv6 |
| **Passwords with `=` or `%`** | `http://user:pass=token%21@proxy:8080` | Preserved without truncation |

---

### 3.4 Automatic Username Country Parsing & `-cc-<country>` Injection

Proxy Guard automatically parses the following patterns in the username string to deduce the proxy's country code:

| Pattern in Username | Example Username | Inferred Country |
| :--- | :--- | :---: |
| `-cc-<iso>` | `customer-resi_xyz-cc-vn` | `"VN"` |
| `_cc_<iso>` | `user_cc_us` | `"US"` |
| `-country-<iso>` | `client-country-de` | `"DE"` |
| `_country-<iso>` | `client_country-sg` | `"SG"` |
| `-zone-<iso>` | `bot-zone-jp` | `"JP"` |

**Clean Pool + `-cc-<country>` Injection (`POST /config` & `POST /proxies/add`)**:
You can keep every line in `upstream_proxy_pool` clean (`http://username:password@host:port`) without manually editing each username:
- Pass `"default_country": "-cc-vn"` (or `"default_country": "VN"` with `"append_cc_to_username": true`) in `POST /config`, or `"country": "-cc-vn"` in `POST /proxies/add`.
- Proxy Guard automatically injects `-cc-vn` into each proxy's username (`http://username-cc-vn:password@host:port`), updates its unique ID, and deduplicates duplicate entries.

---

## 4. Proxy Management Endpoints

### 4.1 `GET /proxies/list`
Returns all registered upstream proxies with passwords masked.

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "strategy": "lowest_failures",
  "total_proxies": 2,
  "proxies": [
    {
      "id": "pr.oxylabs.io:7777",
      "name": "pr.oxylabs.io:7777",
      "protocol": "http",
      "host": "pr.oxylabs.io",
      "port": 7777,
      "auth": {
        "username": "customer-resi_e58ec6d4-cc-vn",
        "password": "***"
      },
      "country": "VN",
      "tags": ["residential"],
      "weight": 1,
      "enabled": true,
      "success_count": 150,
      "failure_count": 0,
      "url": "http://customer-resi_e58ec6d4-cc-vn:***@pr.oxylabs.io:7777"
    }
  ]
}
```

---

### 4.2 `POST /proxies/remove`
Removes an upstream proxy from the active pool.

#### Request Payload:
```json
{
  "id": "pr.oxylabs.io:7777"
}
```

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "message": "Proxy removed successfully"
}
```

---

### 4.3 `POST /proxies/strategy`
Updates the global rotation strategy for the pool.

#### Request Payload:
```json
{
  "strategy": "lowest_failures"
}
```
*Valid values*: `"round_robin"`, `"sticky_session"`, `"lowest_failures"`, `"random"`.

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "strategy": "lowest_failures"
}
```

---

### 4.4 `POST /proxies/report`
Reports proxy health feedback (`success: true | false`) from clients so `lowest_failures` routing avoids blocked or failing proxies.

#### Request Payload:
```json
{
  "id": "pr.oxylabs.io:7777:customer-resi_f4012494",
  "success": false
}
```

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "id": "pr.oxylabs.io:7777:customer-resi_f4012494",
  "success": false,
  "message": "Proxy health result recorded"
}
```

---

## 5. Traffic Optimization & Audit Logs

### 5.1 `GET /logs` & Request Reduction %
Retrieves live entries from the in-memory circular ring buffer (up to 100 entries).

#### Query Parameters:
- `limit` (integer, optional, default `50`): Maximum entries to return.

#### Response (HTTP 200):
```json
{
  "status": "success",
  "request_reduction_percentage": 78.5,
  "blocked_requests": 157,
  "forwarded_requests": 43,
  "total_requests": 200,
  "logs": [
    {
      "id": 1,
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
      "id": 2,
      "timestamp": 1700923049,
      "method": "GET",
      "host": "www.google-analytics.com",
      "path": "/analytics.js",
      "action": "Mocked (JS Stub)",
      "category": "Analytics & Trackers",
      "bytes_saved": 47104,
      "is_blocked": true
    }
  ]
}
```

---

### 5.2 `POST /logs/clear`
Clears the in-memory audit logs ring buffer.

#### Response (HTTP 200):
```json
{
  "status": "ok",
  "message": "Request logs cleared successfully"
}
```

---

### 5.3 `GET /metrics` & `POST /metrics/reset`
Returns cumulative counters for bandwidth, requests, and cost savings.

#### Response (HTTP 200):
```json
{
  "total_requests": 1520,
  "blocked_requests": 1180,
  "forwarded_requests": 340,
  "bytes_saved": 142606336,
  "upstream_bytes_used": 15728640,
  "gb_saved": 0.1328,
  "gb_upstream_used": 0.0146,
  "gb_total_original": 0.1475,
  "data_reduction_percentage": 90.1,
  "estimated_cost_saved_usd": 0.4991,
  "request_reduction_percentage": 77.63
}
```

---

### 5.4 `POST /evaluate`
Simulates whether a single URL will be blocked, mocked, or forwarded before executing a real request.

#### Request Payload:
```json
{
  "url": "https://www.google-analytics.com/analytics.js",
  "method": "GET",
  "resource_type": "script"
}
```

#### Response (HTTP 200):
```json
{
  "action": "Mock",
  "category": "Analytics & Trackers",
  "bytes_saved_est": 47104,
  "mock_content_type": "application/javascript",
  "rule_matched": "google-analytics.com"
}
```

---

### 5.5 `POST /optimize-flow`
Analyzes a full array of HTTP requests or a HAR 1.2 log to report estimated total bandwidth and cost savings.

#### Request Payload:
```json
{
  "requests": [
    { "url": "https://ciscobinary.openh264.org/openh264.zip", "size": 524288 },
    { "url": "https://googletagmanager.com/gtm.js?id=GTM-XYZ", "size": 122880 },
    { "url": "https://quotes.toscrape.com/page/1", "size": 14336 }
  ],
  "cost_per_gb_usd": 3.50
}
```

#### Response (HTTP 200):
```json
{
  "total_requests": 3,
  "blocked_or_mocked_count": 2,
  "forwarded_count": 1,
  "request_reduction_percentage": 66.67,
  "original_bytes": 661504,
  "optimized_bytes": 14336,
  "bytes_saved": 647168,
  "cost_saved_usd": 0.00216
}
```

---

## 6. MCP (Model Context Protocol) Tool Reference

When using AI agents (Gemini, Antigravity, Claude, OpenAI), Proxy Guard registers native MCP tools under the `proxy-guard__` namespace.

| MCP Tool Name | Arguments Schema | Expected Output |
| :--- | :--- | :--- |
| `proxy-guard__get_next_proxy` | `{"country": "VN", "strategy": "lowest_failures", "session_id": "profile-1"}` | Leased proxy credentials and Playwright launch config. |
| `proxy-guard__add_upstream_proxy` | `{"url": "...", "name": "...", "country": "US", "tags": ["res"]}` | Registration confirmation with proxy ID. |
| `proxy-guard__list_upstream_proxies` | `{}` | JSON array of active proxies with masked secrets. |
| `proxy-guard__remove_upstream_proxy` | `{"id": "pr.oxylabs.io:7777"}` | Confirmation message. |
| `proxy-guard__set_rotation_strategy` | `{"strategy": "round_robin"}` | Updated strategy acknowledgment. |
| `proxy-guard__get_request_logs` | `{"limit": 25}` | Ring-buffer logs and request reduction percentage. |
| `proxy-guard__get_metrics` | `{}` | Cumulative metrics counters and dollar savings. |
| `proxy-guard__evaluate_url` | `{"url": "...", "method": "GET"}` | Diagnostic match report (`block_204`, `mock_script`, `bypass_direct`, `forward_upstream`). |
| `proxy-guard__configure_rules` | `{"add_pool_routes": [{"domain": "services.sheerid.com", "pool": "residential"}], "add_direct_bypass_domains": ["cdn.jsdelivr.net"], "default_route": "direct", "cache": {"enabled": true}}` | Updates category toggles, custom block/whitelist/direct-bypass rules, named pool split-routing, fallback route, and bounded HTTP response cache. |

---

## 7. Error Codes & Troubleshooting Guide

| HTTP Status | Error Body / Cause | Solution |
| :---: | :--- | :--- |
| **404** | `{"error": "No healthy upstream proxy available in pool"}` | Pool is empty or all registered proxies are offline. Add proxies via UI (`/custom/proxy-guard/ui`), Settings (`/modules/proxy-guard/config`), or API (`POST /proxies/add`). Ensure client catches 404 for local fallback. |
| **400** | `{"error": "Missing required 'url' field"}` | The payload sent to `/proxies/add` did not include a valid `"url"` string. |
| **400** | `{"error": "Missing required 'id' field"}` | The payload sent to `/proxies/remove` did not include the `"id"` string. |
| **400** | `{"error": "Invalid proxy definition: ..."}` | The proxy string could not be parsed into a valid URI or vendor `host:port:user:pass` pattern. |
| **404** | `{"error": "Not Found: GET /custom/proxy-guard/proxies/next?country=VN"}` | Previous gateway versions did not strip query strings during route lookup. Resolved in v1.5.1+. Alternatively, send `POST /custom/proxy-guard/proxies/next` with a JSON payload. |
