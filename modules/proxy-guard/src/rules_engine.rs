//! Rules Matching Engine and Builtin Catalog.
//!
//! Clean Architecture: Domain Rules Engine.
//! Contains comprehensive, battle-tested filters for high-impact bandwidth waste.

use crate::domain::{FilterDecision, PatternType, ResourceType, Rule, RuleAction, RuleCategory};
use crate::mock_factory::MockFactory;

pub struct RulesEngine {
    rules: Vec<Rule>,
    whitelist: Vec<String>,
}

impl RulesEngine {
    pub fn new(rules: Vec<Rule>, whitelist: Vec<String>) -> Self {
        Self { rules, whitelist }
    }

    pub fn with_builtin_rules() -> Self {
        Self::new(Self::builtin_rules(), Vec::new())
    }

    /// Evaluates a request against whitelist and active rules.
    pub fn evaluate(&self, url: &str, method: &str, resource_hint: Option<&str>) -> FilterDecision {
        let clean_url = url.trim();
        let host = extract_host(clean_url);
        let resource_type = ResourceType::infer(clean_url, resource_hint);
        let method_upper = method.to_ascii_uppercase();

        // 1. Whitelist check (overrides all blocking rules)
        for wl in &self.whitelist {
            let clean_wl = wl.trim().to_ascii_lowercase();
            if host.eq_ignore_ascii_case(&clean_wl)
                || host.to_ascii_lowercase().ends_with(&format!(".{clean_wl}"))
            {
                return FilterDecision {
                    url: clean_url.to_string(),
                    method: method_upper,
                    resource_type,
                    action: RuleAction::ForwardUpstream,
                    matched_rule_id: None,
                    category: None,
                    estimated_bytes_saved: 0,
                    mock: None,
                    reason: format!("Host '{host}' is explicitly whitelisted"),
                };
            }
        }

        // 2. Iterate through rules sorted by priority (higher priority first)
        let mut sorted_rules = self.rules.clone();
        sorted_rules.sort_by_key(|a| std::cmp::Reverse(a.priority));

        for rule in &sorted_rules {
            if rule.matches(&host, clean_url) {
                let mock = MockFactory::create_mock(&rule.action, clean_url);
                let saved = match rule.category {
                    RuleCategory::HeavyMedia => resource_type.typical_byte_size(),
                    RuleCategory::Font => 120_000,
                    RuleCategory::Analytics
                    | RuleCategory::Advertising
                    | RuleCategory::Telemetry => match resource_type {
                        ResourceType::Script => 85_000,
                        ResourceType::Beacon | ResourceType::Image => 2_500,
                        _ => 15_000,
                    },
                    RuleCategory::Custom => 20_000,
                };

                return FilterDecision {
                    url: clean_url.to_string(),
                    method: method_upper,
                    resource_type,
                    action: rule.action.clone(),
                    matched_rule_id: Some(rule.id.clone()),
                    category: Some(rule.category),
                    estimated_bytes_saved: saved,
                    mock,
                    reason: format!("Matched rule '{}' ({:?})", rule.name, rule.category),
                };
            }
        }

        // 3. Fallthrough: forward to upstream proxy
        FilterDecision {
            url: clean_url.to_string(),
            method: method_upper,
            resource_type,
            action: RuleAction::ForwardUpstream,
            matched_rule_id: None,
            category: None,
            estimated_bytes_saved: 0,
            mock: None,
            reason: "No blocking rule matched; forwarded to upstream proxy".to_string(),
        }
    }

    /// Comprehensive built-in rules catalog.
    pub fn builtin_rules() -> Vec<Rule> {
        vec![
            // ── Google Analytics & Tag Manager ──────────────────────────────────
            Rule::new(
                "ga-gtm-js",
                "Google Tag Manager Script",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "googletagmanager.com",
                RuleAction::MockScript,
                100,
                "Mocks GTM script to prevent client script errors while zeroing traffic",
            ),
            Rule::new(
                "ga-analytics-domain",
                "Google Analytics Domain",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "google-analytics.com",
                RuleAction::MockScript,
                95,
                "Mocks Google Analytics tracking domain",
            ),
            Rule::new(
                "ga-analytics-v4",
                "Google Analytics v4 / gtag.js",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "analytics.google.com",
                RuleAction::MockScript,
                95,
                "Mocks GA4 measurement endpoints",
            ),
            Rule::new(
                "ga-collect-beacon",
                "Google Analytics Collect Beacon",
                RuleCategory::Analytics,
                PatternType::UrlContains,
                "/collect?",
                RuleAction::Block204,
                98,
                "Returns 204 No Content for Google Analytics collect beacon pings",
            ),
            Rule::new(
                "ga-g-collect",
                "Google Analytics g/collect Beacon",
                RuleCategory::Analytics,
                PatternType::UrlContains,
                "/g/collect",
                RuleAction::Block204,
                98,
                "Blocks GA4 g/collect measurement protocol requests",
            ),
            Rule::new(
                "ga-doubleclick",
                "Google DoubleClick Ads",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "doubleclick.net",
                RuleAction::Block204,
                90,
                "Blocks DoubleClick ad serving and tracking",
            ),
            Rule::new(
                "ga-syndication",
                "Google Syndication Ads",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "googlesyndication.com",
                RuleAction::Block204,
                90,
                "Blocks Google AdSense and Syndication networks",
            ),
            Rule::new(
                "ga-adservices",
                "Google AdServices",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "googleadservices.com",
                RuleAction::Block204,
                90,
                "Blocks Google Ads conversion tracking",
            ),
            // ── Meta / Facebook ─────────────────────────────────────────────────
            Rule::new(
                "fb-pixel-net",
                "Meta Facebook Connect & Pixel",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "connect.facebook.net",
                RuleAction::MockScript,
                95,
                "Mocks Facebook fbevents.js to prevent fbq errors",
            ),
            Rule::new(
                "fb-pixel-domain",
                "Meta Pixel Domain",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "pixel.facebook.com",
                RuleAction::MockPixel,
                92,
                "Returns 1x1 GIF for Facebook tracking pixel",
            ),
            Rule::new(
                "fb-graph-api-events",
                "Meta Graph API Events",
                RuleCategory::Analytics,
                PatternType::UrlContains,
                "graph.facebook.com/v",
                RuleAction::BlockEmptyJson,
                90,
                "Blocks Meta Graph API event tracking",
            ),
            // ── TikTok ──────────────────────────────────────────────────────────
            Rule::new(
                "tiktok-analytics",
                "TikTok Analytics & Pixel",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "analytics.tiktok.com",
                RuleAction::MockScript,
                90,
                "Mocks TikTok Pixel scripts",
            ),
            Rule::new(
                "tiktok-mon",
                "TikTok Monitor Endpoint",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "mon.tiktok.com",
                RuleAction::Block204,
                88,
                "Blocks TikTok monitoring beacons",
            ),
            // ── Snapchat ─────────────────────────────────────────────────────────
            Rule::new(
                "snapchat-pixel",
                "Snapchat Pixel",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "sc-static.net",
                RuleAction::Block204,
                85,
                "Blocks Snapchat conversion tracking pixel",
            ),
            // ── Pinterest ─────────────────────────────────────────────────────────
            Rule::new(
                "pinterest-pixel",
                "Pinterest Conversion Tracking",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "pintrk.com",
                RuleAction::Block204,
                85,
                "Blocks Pinterest conversion tracking tag",
            ),
            Rule::new(
                "pinterest-ct",
                "Pinterest Analytics",
                RuleCategory::Analytics,
                PatternType::UrlContains,
                "ct.pinterest.com",
                RuleAction::MockPixel,
                85,
                "Blocks Pinterest analytics events",
            ),
            // ── LinkedIn ─────────────────────────────────────────────────────────
            Rule::new(
                "linkedin-insight",
                "LinkedIn Insight Tag",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "snap.licdn.com",
                RuleAction::Block204,
                85,
                "Blocks LinkedIn Insight tag scripts",
            ),
            // ── Twitter / X ──────────────────────────────────────────────────────
            Rule::new(
                "twitter-pixel",
                "Twitter/X Universal Website Tag",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "static.ads-twitter.com",
                RuleAction::Block204,
                85,
                "Blocks Twitter/X Ads conversion tracking",
            ),
            // ── Heatmaps & Session Replays ───────────────────────────────────────
            Rule::new(
                "hotjar",
                "Hotjar Heatmap & Recording",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "hotjar.com",
                RuleAction::MockScript,
                88,
                "Blocks Hotjar recording scripts and websocket connections",
            ),
            Rule::new(
                "hotjar-static",
                "Hotjar Static Assets",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "static.hotjar.com",
                RuleAction::Block204,
                88,
                "Blocks Hotjar static assets",
            ),
            Rule::new(
                "clarity-ms",
                "Microsoft Clarity Session Recording",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "clarity.ms",
                RuleAction::MockScript,
                88,
                "Blocks Microsoft Clarity session telemetry and recordings",
            ),
            Rule::new(
                "smartlook",
                "Smartlook Session Replay",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "smartlook.com",
                RuleAction::Block204,
                85,
                "Blocks Smartlook session recording",
            ),
            Rule::new(
                "fullstory",
                "FullStory Session Tracking",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "fullstory.com",
                RuleAction::Block204,
                85,
                "Blocks FullStory session recordings",
            ),
            Rule::new(
                "logrocket",
                "LogRocket Session Replay",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "logrocket.io",
                RuleAction::Block204,
                85,
                "Blocks LogRocket session replay and telemetry",
            ),
            Rule::new(
                "luckyorange",
                "Lucky Orange Heatmap",
                RuleCategory::Analytics,
                PatternType::DomainSuffix,
                "luckyorange.com",
                RuleAction::Block204,
                85,
                "Blocks Lucky Orange session recording",
            ),
            // ── Telemetry & Error Tracking ───────────────────────────────────────
            Rule::new(
                "sentry",
                "Sentry Error Telemetry",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "sentry.io",
                RuleAction::MockScript,
                85,
                "Mocks Sentry SDK to drop error beacon reports",
            ),
            Rule::new(
                "sentry-cdn",
                "Sentry Browser CDN",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "browser.sentry-cdn.com",
                RuleAction::MockScript,
                85,
                "Mocks Sentry browser SDK bundle",
            ),
            Rule::new(
                "datadog-rum",
                "Datadog RUM Telemetry",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "browser-intake-datadoghq.com",
                RuleAction::Block204,
                85,
                "Blocks Datadog Real User Monitoring beacons",
            ),
            Rule::new(
                "newrelic",
                "New Relic Browser Agent",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "newrelic.com",
                RuleAction::MockScript,
                84,
                "Mocks New Relic browser agent to suppress telemetry uploads",
            ),
            Rule::new(
                "newrelic-bam",
                "New Relic BAM Beacon",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "bam.nr-data.net",
                RuleAction::Block204,
                84,
                "Blocks New Relic browser-application monitoring beacons",
            ),
            Rule::new(
                "segment-telemetry",
                "Segment Customer Data Platform",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "segment.io",
                RuleAction::BlockEmptyJson,
                85,
                "Returns empty JSON for Segment analytics pings",
            ),
            Rule::new(
                "segment-cdn",
                "Segment Analytics CDN",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "cdn.segment.com",
                RuleAction::Block204,
                85,
                "Blocks Segment analytics.js download",
            ),
            Rule::new(
                "amplitude",
                "Amplitude Product Analytics",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "amplitude.com",
                RuleAction::BlockEmptyJson,
                85,
                "Blocks Amplitude telemetry events",
            ),
            Rule::new(
                "mixpanel",
                "Mixpanel Analytics",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "mixpanel.com",
                RuleAction::BlockEmptyJson,
                85,
                "Blocks Mixpanel product analytics events",
            ),
            Rule::new(
                "heap-analytics",
                "Heap.io Analytics",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "heapanalytics.com",
                RuleAction::Block204,
                84,
                "Blocks Heap auto-capture analytics",
            ),
            Rule::new(
                "posthog",
                "PostHog Product Analytics",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "app.posthog.com",
                RuleAction::BlockEmptyJson,
                84,
                "Blocks PostHog analytics events",
            ),
            Rule::new(
                "intercom",
                "Intercom Widget",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "intercom.io",
                RuleAction::Block204,
                82,
                "Blocks Intercom live chat widget and analytics",
            ),
            Rule::new(
                "zendesk-widget",
                "Zendesk Chat Widget",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "static.zdassets.com",
                RuleAction::Block204,
                80,
                "Blocks Zendesk customer chat widget scripts",
            ),
            // ── Ad Exchanges & Content Recommendation ────────────────────────────
            Rule::new(
                "criteo",
                "Criteo Retargeting",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "criteo.com",
                RuleAction::Block204,
                80,
                "Blocks Criteo retargeting ads and tracking",
            ),
            Rule::new(
                "taboola",
                "Taboola Sponsored Content",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "taboola.com",
                RuleAction::Block204,
                80,
                "Blocks Taboola content recommendation widgets",
            ),
            Rule::new(
                "outbrain",
                "Outbrain Sponsored Content",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "outbrain.com",
                RuleAction::Block204,
                80,
                "Blocks Outbrain content widgets",
            ),
            Rule::new(
                "adroll",
                "AdRoll Retargeting",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "adroll.com",
                RuleAction::Block204,
                78,
                "Blocks AdRoll retargeting pixel and scripts",
            ),
            Rule::new(
                "pubmatic",
                "PubMatic Ad Exchange",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "pubmatic.com",
                RuleAction::Block204,
                78,
                "Blocks PubMatic programmatic ad exchange",
            ),
            Rule::new(
                "rubiconproject",
                "Rubicon Project (Magnite) DSP",
                RuleCategory::Advertising,
                PatternType::DomainSuffix,
                "rubiconproject.com",
                RuleAction::Block204,
                78,
                "Blocks Magnite (Rubicon) demand-side platform",
            ),
            // ── Heavy Media (Videos & Streams) ───────────────────────────────────
            Rule::new(
                "media-mp4",
                "MP4 Video Files",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".mp4",
                RuleAction::Block204,
                70,
                "Blocks video downloads saving 5-50MB per video",
            ),
            Rule::new(
                "media-webm",
                "WebM Video Files",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".webm",
                RuleAction::Block204,
                70,
                "Blocks WebM video streaming assets",
            ),
            Rule::new(
                "media-m3u8",
                "HLS Playlist Files",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".m3u8",
                RuleAction::Block204,
                70,
                "Blocks HLS video playlist files",
            ),
            Rule::new(
                "media-ts-segment",
                "HLS Video Segments",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".ts",
                RuleAction::Block204,
                70,
                "Blocks HLS transport stream video segment files",
            ),
            Rule::new(
                "media-mov",
                "QuickTime Video Files",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".mov",
                RuleAction::Block204,
                70,
                "Blocks QuickTime video downloads",
            ),
            Rule::new(
                "media-avi",
                "AVI Video Files",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".avi",
                RuleAction::Block204,
                70,
                "Blocks AVI video files",
            ),
            // ── Heavy Web Fonts ───────────────────────────────────────────────────
            Rule::new(
                "fonts-woff2",
                "WOFF2 Font Files",
                RuleCategory::Font,
                PatternType::Extension,
                ".woff2",
                RuleAction::Block204,
                65,
                "Blocks custom WOFF2 font downloads",
            ),
            Rule::new(
                "fonts-woff",
                "WOFF Font Files",
                RuleCategory::Font,
                PatternType::Extension,
                ".woff",
                RuleAction::Block204,
                65,
                "Blocks custom WOFF font downloads",
            ),
            Rule::new(
                "fonts-ttf",
                "TTF Font Files",
                RuleCategory::Font,
                PatternType::Extension,
                ".ttf",
                RuleAction::Block204,
                65,
                "Blocks custom TTF font downloads",
            ),
            Rule::new(
                "fonts-eot",
                "EOT Font Files",
                RuleCategory::Font,
                PatternType::Extension,
                ".eot",
                RuleAction::Block204,
                65,
                "Blocks legacy EOT font downloads",
            ),
            Rule::new(
                "fonts-otf",
                "OTF Font Files",
                RuleCategory::Font,
                PatternType::Extension,
                ".otf",
                RuleAction::Block204,
                65,
                "Blocks OpenType OTF font downloads",
            ),
            Rule::new(
                "fonts-google",
                "Google Fonts CDN",
                RuleCategory::Font,
                PatternType::DomainSuffix,
                "fonts.googleapis.com",
                RuleAction::Block204,
                68,
                "Blocks Google Fonts CSS stylesheet fetches",
            ),
            Rule::new(
                "fonts-gstatic",
                "Google Static Font Files",
                RuleCategory::Font,
                PatternType::DomainSuffix,
                "fonts.gstatic.com",
                RuleAction::Block204,
                68,
                "Blocks Google Fonts binary file downloads",
            ),
            // ── Waste Assets (contribute nothing to automation flows) ─────────────
            Rule::new(
                "favicon",
                "Favicon ICO Files",
                RuleCategory::HeavyMedia,
                PatternType::UrlContains,
                "favicon.ico",
                RuleAction::Block204,
                60,
                "Blocks favicon downloads that bots never render",
            ),
            Rule::new(
                "service-worker",
                "Service Worker Scripts",
                RuleCategory::Telemetry,
                PatternType::UrlContains,
                "sw.js",
                RuleAction::Block204,
                60,
                "Blocks service worker registration that can trigger background sync",
            ),
            Rule::new(
                "web-manifest",
                "Web App Manifest",
                RuleCategory::HeavyMedia,
                PatternType::UrlContains,
                "manifest.json",
                RuleAction::BlockEmptyJson,
                58,
                "Returns minimal empty JSON for web app manifests",
            ),
            Rule::new(
                "web-manifest-webmanifest",
                "WebApp Manifest (.webmanifest)",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".webmanifest",
                RuleAction::BlockEmptyJson,
                58,
                "Returns minimal empty JSON for .webmanifest files",
            ),
            Rule::new(
                "sourcemaps",
                "JavaScript Source Maps",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".js.map",
                RuleAction::Block204,
                60,
                "Blocks JS source map downloads (dev-only, never needed for automation)",
            ),
            Rule::new(
                "css-sourcemaps",
                "CSS Source Maps",
                RuleCategory::HeavyMedia,
                PatternType::Extension,
                ".css.map",
                RuleAction::Block204,
                58,
                "Blocks CSS source map downloads",
            ),
        ]
    }
}

/// Helper to parse host from full URL.
pub fn extract_host(url: &str) -> String {
    let without_proto = if let Some(idx) = url.find("://") {
        &url[idx + 3..]
    } else {
        url
    };

    let host_part = without_proto.split('/').next().unwrap_or(without_proto);
    let host_no_port = host_part.split(':').next().unwrap_or(host_part);
    host_no_port.to_ascii_lowercase()
}
