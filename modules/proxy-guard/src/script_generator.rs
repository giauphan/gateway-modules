//! Client Interception Script Generators.
//!
//! Clean Architecture: Application Service / Code Generator.
//! Exports production-ready Playwright, Puppeteer, and mitmproxy interceptors.

pub struct ScriptGenerator;

impl ScriptGenerator {
    /// Generates Playwright TypeScript route interception code.
    pub fn playwright_ts() -> String {
        r#"// [Proxy Guard] Zero-Bandwidth Playwright Interceptor (TypeScript/Node.js)
// Drop this into your browser context setup to save 90-95% proxy bandwidth!

import { BrowserContext, Route } from 'playwright';

const BLOCKED_DOMAINS = [
  'google-analytics.com', 'googletagmanager.com', 'doubleclick.net', 'googlesyndication.com',
  'connect.facebook.net', 'pixel.facebook.com', 'analytics.tiktok.com',
  'hotjar.com', 'clarity.ms', 'smartlook.com', 'fullstory.com',
  'sentry.io', 'browser-intake-datadoghq.com', 'segment.io', 'amplitude.com',
  'criteo.com', 'taboola.com', 'outbrain.com'
];

const BLOCKED_EXTENSIONS = ['.mp4', '.webm', '.ogg', '.woff2', '.woff', '.ttf'];

export async function setupProxyGuardInterceptor(context: BrowserContext, blockImages = true) {
  await context.route('**/*', (route: Route) => {
    const request = route.request();
    const url = request.url();
    const resourceType = request.resourceType();

    // 1. Block heavy resource types
    if (['media', 'font'].includes(resourceType) || (blockImages && resourceType === 'image')) {
      return route.abort(); // 0 bytes hit your paid proxy!
    }

    // 2. Block heavy file extensions
    const cleanUrl = url.split('?')[0].toLowerCase();
    if (BLOCKED_EXTENSIONS.some(ext => cleanUrl.endsWith(ext))) {
      return route.abort();
    }

    // 3. Block tracking and analytics domains with harmless stubs
    const host = new URL(url).hostname.toLowerCase();
    if (BLOCKED_DOMAINS.some(domain => host === domain || host.endsWith('.' + domain))) {
      if (resourceType === 'script') {
        return route.fulfill({
          status: 200,
          contentType: 'application/javascript',
          body: 'window.dataLayer = window.dataLayer || []; window.gtag = function(){}; window.fbq = function(){};'
        });
      }
      return route.fulfill({ status: 204, body: '' });
    }

    // 4. Allowed requests continue through your paid proxy
    return route.continue();
  });
}
"#.to_string()
    }

    /// Generates Playwright Python route interception code.
    pub fn playwright_py() -> String {
        r#"# [Proxy Guard] Zero-Bandwidth Playwright Interceptor (Python)
# Plug this into your playwright flow before navigating!

from urllib.parse import urlparse

BLOCKED_DOMAINS = {
    "google-analytics.com", "googletagmanager.com", "doubleclick.net", "googlesyndication.com",
    "connect.facebook.net", "pixel.facebook.com", "analytics.tiktok.com",
    "hotjar.com", "clarity.ms", "smartlook.com", "fullstory.com",
    "sentry.io", "browser-intake-datadoghq.com", "segment.io", "amplitude.com",
    "criteo.com", "taboola.com", "outbrain.com"
}

BLOCKED_EXTENSIONS = (".mp4", ".webm", ".ogg", ".woff2", ".woff", ".ttf")

def setup_proxy_guard_interceptor(context, block_images=True):
    def handle_route(route):
        request = route.request
        url = request.url
        resource_type = request.resource_type

        # 1. Block heavy resource types
        if resource_type in ("media", "font") or (block_images and resource_type == "image"):
            route.abort()
            return

        # 2. Block extensions
        clean_url = url.split("?")[0].lower()
        if clean_url.endswith(BLOCKED_EXTENSIONS):
            route.abort()
            return

        # 3. Block analytics/ads domains with harmless stub
        hostname = urlparse(url).hostname or ""
        hostname = hostname.lower()
        if any(hostname == d or hostname.endswith("." + d) for d in BLOCKED_DOMAINS):
            if resource_type == "script":
                route.fulfill(
                    status=200,
                    content_type="application/javascript",
                    body="window.dataLayer = window.dataLayer || []; window.gtag = function(){}; window.fbq = function(){};"
                )
            else:
                route.fulfill(status=204, body="")
            return

        route.continue_()

    context.route("**/*", handle_route)
"#.to_string()
    }

    /// Generates Puppeteer Node.js route interception code.
    pub fn puppeteer_ts() -> String {
        r#"// [Proxy Guard] Zero-Bandwidth Puppeteer Interceptor (Node.js)
import { Page, HTTPRequest } from 'puppeteer';

const BLOCKED_DOMAINS = [
  'google-analytics.com', 'googletagmanager.com', 'doubleclick.net',
  'connect.facebook.net', 'pixel.facebook.com', 'hotjar.com', 'clarity.ms', 'sentry.io'
];

export async function setupPuppeteerProxyGuard(page: Page, blockImages = true) {
  await page.setRequestInterception(true);

  page.on('request', (request: HTTPRequest) => {
    const url = request.url();
    const type = request.resourceType();

    if (['media', 'font'].includes(type) || (blockImages && type === 'image')) {
      return request.abort();
    }

    const host = new URL(url).hostname.toLowerCase();
    if (BLOCKED_DOMAINS.some(d => host === d || host.endsWith('.' + d))) {
      if (type === 'script') {
        return request.respond({
          status: 200,
          contentType: 'application/javascript',
          body: 'window.dataLayer = window.dataLayer || []; window.gtag = function(){};'
        });
      }
      return request.respond({ status: 204, body: '' });
    }

    return request.continue();
  });
}
"#
        .to_string()
    }

    /// Generates mitmproxy Python addon script.
    pub fn mitmproxy_py() -> String {
        r#"# [Proxy Guard] mitmproxy Bandwidth Saver Addon (Python)
# Usage: mitmproxy -s proxy_guard_addon.py --mode upstream:http://upstream-proxy:8080 --set upstream_auth=user:pass

from mitmproxy import http

BLOCKED_DOMAINS = {
    "google-analytics.com", "googletagmanager.com", "doubleclick.net", "googlesyndication.com",
    "connect.facebook.net", "pixel.facebook.com", "analytics.tiktok.com",
    "hotjar.com", "clarity.ms", "smartlook.com", "fullstory.com",
    "sentry.io", "browser-intake-datadoghq.com", "segment.io", "amplitude.com"
}

BLOCKED_EXTENSIONS = (".mp4", ".webm", ".woff2", ".woff", ".ttf")

class ProxyGuard:
    def request(self, flow: http.HTTPFlow) -> None:
        host = flow.request.pretty_host.lower()
        path = flow.request.path.lower()

        # Check domain suffix
        if any(host == d or host.endswith("." + d) for d in BLOCKED_DOMAINS):
            flow.response = http.Response.make(
                204,
                b"",
                {"Content-Type": "text/plain", "X-Proxy-Guard": "Blocked-Analytics"}
            )
            return

        # Check media extensions
        clean_path = path.split("?")[0]
        if clean_path.endswith(BLOCKED_EXTENSIONS):
            flow.response = http.Response.make(
                204,
                b"",
                {"Content-Type": "text/plain", "X-Proxy-Guard": "Blocked-Media"}
            )
            return

addons = [ProxyGuard()]
"#
        .to_string()
    }
}
