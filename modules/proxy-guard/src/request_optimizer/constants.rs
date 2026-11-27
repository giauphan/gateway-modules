//! Constants for Query Parameters and Request Headers to strip.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.

/// Tracking-only query parameters that should be stripped before forwarding.
/// These contribute zero value to automation flows but can add hundreds of bytes to URLs
/// and cause tracking systems to wake up and phone home.
pub static STRIPPED_QUERY_PARAMS: &[&str] = &[
    // Google / Universal Analytics
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "utm_source_platform",
    "utm_creative_format",
    "utm_marketing_tactic",
    // Google Ads
    "gclid",
    "gclsrc",
    "gbraid",
    "wbraid",
    "dclid",
    // Meta / Facebook
    "fbclid",
    "fb_action_ids",
    "fb_action_types",
    "fb_source",
    "fb_ref",
    // TikTok & Microsoft
    "ttclid",
    "msclkid",
    // HubSpot
    "_hsenc",
    "_hsmi",
    "hsa_acc",
    "hsa_cam",
    "hsa_grp",
    "hsa_ad",
    "hsa_src",
    "hsa_tgt",
    "hsa_kw",
    "hsa_mt",
    "hsa_net",
    "hsa_ver",
    // Email & CRM tracking
    "mc_cid",
    "mc_eid",
    "_kx",
    "__s",
    "vero_id",
    "vero_conv",
    "omnisendcontactid",
    "_itm",
    // Generic click tracking
    "s_kwcid",
    "ef_id",
    "pcrid",
    "pd_rd_r",
    "pd_rd_w",
    "pd_rd_wg",
    "pf_rd_p",
    "pf_rd_r",
];

/// HTTP request headers that should be dropped before forwarding to upstream.
/// These either identify the browser fingerprint, trigger telemetry pipelines,
/// or are unnecessary for headless automation flows.
pub static STRIPPED_REQUEST_HEADERS: &[&str] = &[
    // Browser fingerprinting headers
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "sec-ch-ua-arch",
    "sec-ch-ua-bitness",
    "sec-ch-ua-full-version",
    "sec-ch-ua-full-version-list",
    "sec-ch-ua-model",
    "sec-ch-ua-wow64",
    "sec-ch-prefers-color-scheme",
    "sec-ch-prefers-reduced-motion",
    // Downstream proxy and hop-by-hop headers (RFC 9110)
    "proxy-authorization",
    "proxy-authenticate",
    "proxy-connection",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
    // Internal proxy routing headers (must not leak to upstream targets)
    "x-proxy-session",
    "x-proxy-session-id",
    "x-proxy-pool",
    "x-proxy-country",
    "x-proxy-tag",
    "x-proxy-id",
    "x-proxy-strategy",
    "x-proxy-rotation-strategy",
    // No referrer sent - reduces analytics triggering
    "referer",
    // Privacy / tracking headers
    "x-forwarded-for",
    "x-real-ip",
    "x-request-id",
    "traceparent",
    "tracestate",
    "baggage",
    // Browser performance hints that waste bytes
    "save-data",
    "device-memory",
    "downlink",
    "ect",
    "rtt",
    "viewport-width",
    "width",
    "dpr",
    "content-dpr",
    // Service Worker header (triggers SW registration which downloads more scripts)
    "service-worker",
];
