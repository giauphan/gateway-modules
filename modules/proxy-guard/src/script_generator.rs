//! Client Interception Script Generators.
//!
//! Clean Architecture: Application Service / Code Generator.
//! Exports production-ready Playwright, Puppeteer, and mitmproxy interceptors.

pub struct ScriptGenerator;

impl ScriptGenerator {
    /// Generates Playwright TypeScript persistent-context launcher backed by Proxy Guard.
    /// Does NOT enable `page.route()` by default so Chromium's HTTP disk cache and TLS fingerprint stay intact.
    pub fn playwright_ts() -> String {
        r#"// [Proxy Guard] Bandwidth-Optimized Playwright Launcher (TypeScript/Node.js)
// Preserves Chromium's HTTP disk cache AND original JA3/JA4 TLS fingerprint!
//
// IMPORTANT (HTTP Cache Trap):
// Enabling context.route() / page.route() disables Chromium's HTTP disk cache,
// forcing heavy JS bundles to re-download on every navigation.
// By default (`enablePageRouteBlocking = false`), ALL ad/font/tracker blocking and
// domain split-routing (`Reject` / `Direct` / `Pool`) are offloaded to Proxy Guard
// at the local proxy layer (http://127.0.0.1:3128).

import { chromium, BrowserContext, Route } from 'playwright';

export interface ProxyGuardLaunchOptions {
  /** Local Proxy Guard HTTP/CONNECT listener URL */
  proxyServer?: string;
  /** Persistent profile directory to reuse Chromium's HTTP disk cache across runs */
  userDataDir?: string;
  /** Target pool + sticky session ID passed via Proxy-Authorization (e.g. "residential:sheerid-01") */
  sessionId?: string;
  /** Named upstream pool prefix (e.g. "residential", "static_isp_us") */
  pool?: string;
  /** Align browser timezone with upstream proxy region (e.g. "America/New_York") */
  timezoneId?: string;
  /** Align browser locale with upstream proxy region (e.g. "en-US") */
  locale?: string;
  headless?: boolean;
  /**
   * Opt-in fallback ONLY when no local proxy daemon can run (e.g. serverless).
   * Defaults to `false` to keep Chromium's HTTP disk cache active.
   */
  enablePageRouteBlocking?: boolean;
}

export async function launchWithProxyGuard(
  opts: ProxyGuardLaunchOptions = {}
): Promise<BrowserContext> {
  const {
    proxyServer = 'http://127.0.0.1:3128',
    userDataDir = './cache/pw-profile',
    sessionId = 'sheerid-session-01',
    pool = 'residential',
    timezoneId = 'America/New_York',
    locale = 'en-US',
    headless = true,
    enablePageRouteBlocking = false,
  } = opts;

  const proxyUsername = pool ? `${pool}:${sessionId}` : `session-${sessionId}`;

  const context = await chromium.launchPersistentContext(userDataDir, {
    headless,
    timezoneId,
    locale,
    proxy: {
      server: proxyServer,
      username: proxyUsername,
      password: 'x',
    },
    args: [
      // Prevent browser background update / codec downloads
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-default-apps',
      '--no-default-browser-check',
      '--disable-sync',
      // Prevent WebRTC real-IP leaks outside the proxy tunnel
      '--force-webrtc-ip-handling-policy=disable_non_proxied_udp',
      '--enforce-webrtc-ip-permission-check',
    ],
  });

  if (enablePageRouteBlocking) {
    await attachFallbackRouteBlocker(context);
  }

  return context;
}

const FALLBACK_BLOCKED_EXTENSIONS = ['.mp4', '.webm', '.ogg', '.woff2', '.woff', '.ttf'];

async function attachFallbackRouteBlocker(context: BrowserContext): Promise<void> {
  await context.route('**/*', (route: Route) => {
    const req = route.request();
    const resourceType = req.resourceType();
    const cleanUrl = req.url().split('?')[0].toLowerCase();
    if (
      resourceType === 'media' ||
      resourceType === 'font' ||
      FALLBACK_BLOCKED_EXTENSIONS.some((ext) => cleanUrl.endsWith(ext))
    ) {
      return route.abort();
    }
    return route.continue();
  });
}
"#
        .to_string()
    }

    /// Generates Playwright Python persistent-context launcher backed by Proxy Guard.
    /// Does NOT enable `page.route()` by default so Chromium's HTTP disk cache and TLS fingerprint stay intact.
    pub fn playwright_py() -> String {
        r#"# [Proxy Guard] Bandwidth-Optimized Playwright Launcher (Python)
# Preserves Chromium's HTTP disk cache AND original JA3/JA4 TLS fingerprint!
#
# IMPORTANT (HTTP Cache Trap):
# Enabling context.route() / page.route() disables Chromium's HTTP disk cache,
# forcing heavy JS bundles to re-download on every navigation.
# By default (`enable_page_route_blocking=False`), ALL ad/font/tracker blocking and
# domain split-routing (`Reject` / `Direct` / `Pool`) are offloaded to Proxy Guard
# at the local proxy layer (http://127.0.0.1:3128).

from playwright.sync_api import Playwright, BrowserContext

FALLBACK_BLOCKED_EXTENSIONS = (".mp4", ".webm", ".ogg", ".woff2", ".woff", ".ttf")


def launch_with_proxy_guard(
    playwright: Playwright,
    proxy_server: str = "http://127.0.0.1:3128",
    user_data_dir: str = "./cache/pw-profile",
    session_id: str = "sheerid-session-01",
    pool: str = "residential",
    timezone_id: str = "America/New_York",
    locale: str = "en-US",
    headless: bool = True,
    enable_page_route_blocking: bool = False,
) -> BrowserContext:
    proxy_username = f"{pool}:{session_id}" if pool else f"session-{session_id}"

    context = playwright.chromium.launch_persistent_context(
        user_data_dir=user_data_dir,
        headless=headless,
        timezone_id=timezone_id,
        locale=locale,
        proxy={
            "server": proxy_server,
            "username": proxy_username,
            "password": "x",
        },
        args=[
            # Prevent browser background update / codec downloads
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--no-default-browser-check",
            "--disable-sync",
            # Prevent WebRTC real-IP leaks outside the proxy tunnel
            "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
            "--enforce-webrtc-ip-permission-check",
        ],
    )

    if enable_page_route_blocking:
        _attach_fallback_route_blocker(context)

    return context


def _attach_fallback_route_blocker(context: BrowserContext) -> None:
    def handle_route(route):
        req = route.request
        clean_url = req.url.split("?")[0].lower()
        if req.resource_type in ("media", "font") or clean_url.endswith(
            FALLBACK_BLOCKED_EXTENSIONS
        ):
            route.abort()
            return
        route.continue_()

    context.route("**/*", handle_route)
"#
        .to_string()
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
# WARNING (TLS Fingerprint): mitmproxy terminates client TLS and re-handshakes with OpenSSL.
# Always bypass sensitive anti-bot hosts (services.sheerid.com, muse.ai, auth.meta.com) using
# --ignore-hosts so their original browser TLS fingerprint stays intact, or prefer non-MITM
# domain split-routing via Proxy Guard (127.0.0.1:3128) / mihomo / proxy-chain.
# Usage:
#   mitmproxy -s proxy_guard_addon.py \
#     --mode upstream:http://upstream-proxy:8080 --set upstream_auth=user:pass \
#     --ignore-hosts '^(services\.sheerid\.com|muse\.ai|.*\.muse\.ai|auth\.meta\.com):443$'

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
