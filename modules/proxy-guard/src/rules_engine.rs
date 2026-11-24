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
            // Google Analytics & Tag Manager
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
            // Meta / Facebook
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
            // TikTok
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
            // Heatmaps & Session Replays (Massive bandwidth hogs)
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
            // Telemetry & Error Tracking
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
                "amplitude",
                "Amplitude Product Analytics",
                RuleCategory::Telemetry,
                PatternType::DomainSuffix,
                "amplitude.com",
                RuleAction::BlockEmptyJson,
                85,
                "Blocks Amplitude telemetry events",
            ),
            // Ad Exchanges & Content Recommendation
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
            // Heavy Media (Videos & Streams)
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
            // Heavy Web Fonts
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
