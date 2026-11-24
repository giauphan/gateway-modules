//! Built-in rules for Browser Background Noise, Component Updaters, and Telemetry.
//!
//! Intercepts Google Chrome Widevine/component delivery (gvt1.com), Firefox OpenH264 codec,
//! browser update checks, and client-side feature flag/fraud telemetry.
//! Clean Architecture: Domain Layer (Static Catalog).

use crate::domain::{PatternType, Rule, RuleAction, RuleCategory};

pub fn browser_noise_rules() -> Vec<Rule> {
    vec![
        // ── Chromium / Google Background Noise ────────────────────────────────
        Rule::new(
            "chrome-gvt1-components",
            "Google Component Delivery (Widevine/CRX)",
            RuleCategory::HeavyMedia,
            PatternType::DomainSuffix,
            "gvt1.com",
            RuleAction::Block204,
            85,
            "Blocks Chrome component updater (Widevine, CRLSet) saving 9-20MB per browser profile",
        ),
        Rule::new(
            "chrome-update-api",
            "Google Chrome Update API",
            RuleCategory::Telemetry,
            PatternType::DomainSuffix,
            "update.googleapis.com",
            RuleAction::BlockEmptyJson,
            80,
            "Blocks Chrome background update check beacons",
        ),
        Rule::new(
            "google-dl-updater",
            "Google Download Updater",
            RuleCategory::Telemetry,
            PatternType::DomainSuffix,
            "dl.google.com",
            RuleAction::Block204,
            80,
            "Blocks Google background updater downloads",
        ),
        // ── Firefox Background Noise ──────────────────────────────────────────
        Rule::new(
            "firefox-openh264",
            "Cisco OpenH264 Codec Binary",
            RuleCategory::HeavyMedia,
            PatternType::DomainSuffix,
            "ciscobinary.openh264.org",
            RuleAction::Block204,
            85,
            "Blocks Firefox OpenH264 binary codec download saving ~500KB per profile",
        ),
        Rule::new(
            "firefox-aus-update",
            "Mozilla Firefox Update Service (AUS)",
            RuleCategory::Telemetry,
            PatternType::DomainSuffix,
            "aus5.mozilla.org",
            RuleAction::BlockEmptyJson,
            80,
            "Blocks Firefox application update service requests",
        ),
        // ── Fraud & Feature Flag Telemetry ────────────────────────────────────
        Rule::new(
            "threatmetrix-profiling",
            "LexisNexis ThreatMetrix Fraud Profiling",
            RuleCategory::Telemetry,
            PatternType::DomainSuffix,
            "online-metrix.net",
            RuleAction::Block204,
            75,
            "Blocks ThreatMetrix browser fingerprinting and profiling beacons",
        ),
        Rule::new(
            "launchdarkly-events",
            "LaunchDarkly Feature Flag Telemetry",
            RuleCategory::Telemetry,
            PatternType::DomainSuffix,
            "events.launchdarkly.com",
            RuleAction::Block204,
            75,
            "Blocks LaunchDarkly analytics event beacons",
        ),
    ]
}
