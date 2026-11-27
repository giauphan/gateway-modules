use proxy_guard::domain::{
    DefaultRoute, PatternType, ResourceType, Rule, RuleAction, RuleCategory,
};
use proxy_guard::rules_engine::RulesEngine;

#[test]
fn test_builtin_rules_intercept_analytics() {
    let engine = RulesEngine::with_builtin_rules();

    let d1 = engine.evaluate(
        "https://www.googletagmanager.com/gtm.js?id=GTM-XXXX",
        "GET",
        Some("script"),
    );
    assert!(d1.is_blocked());
    assert_eq!(d1.action, RuleAction::MockScript);
    assert!(d1.mock.is_some());
    assert!(d1.mock.unwrap().body.contains("dataLayer"));

    let d2 = engine.evaluate(
        "https://www.google-analytics.com/g/collect?v=2",
        "POST",
        Some("beacon"),
    );
    assert!(d2.is_blocked());
    assert_eq!(d2.action, RuleAction::Block204);

    let d3 = engine.evaluate(
        "https://connect.facebook.net/en_US/fbevents.js",
        "GET",
        Some("script"),
    );
    assert!(d3.is_blocked());
    assert_eq!(d3.action, RuleAction::MockScript);
    assert!(d3.mock.unwrap().body.contains("fbq"));

    let d4 = engine.evaluate(
        "https://static.hotjar.com/c/hotjar-12345.js",
        "GET",
        Some("script"),
    );
    assert!(d4.is_blocked());

    let d5 = engine.evaluate(
        "https://o12345.ingest.sentry.io/api/12345/envelope/",
        "POST",
        Some("xhr"),
    );
    assert!(d5.is_blocked());
}

#[test]
fn test_builtin_rules_intercept_heavy_media_and_fonts() {
    let engine = RulesEngine::with_builtin_rules();

    let d_video = engine.evaluate(
        "https://cdn.example.com/videos/promo_4k.mp4",
        "GET",
        Some("media"),
    );
    assert!(d_video.is_blocked());
    assert_eq!(d_video.action, RuleAction::Block204);
    assert_eq!(d_video.resource_type, ResourceType::MediaVideo);
    assert!(d_video.estimated_bytes_saved >= 5_000_000);

    let d_font = engine.evaluate(
        "https://cdn.example.com/fonts/inter-bold.woff2",
        "GET",
        Some("font"),
    );
    assert!(d_font.is_blocked());
    assert_eq!(d_font.action, RuleAction::Block204);
}

#[test]
fn test_whitelist_bypasses_blocking_rules() {
    let engine = RulesEngine::new(
        RulesEngine::builtin_rules(),
        vec!["google-analytics.com".to_string()],
    );

    let d = engine.evaluate(
        "https://www.google-analytics.com/analytics.js",
        "GET",
        Some("script"),
    );
    assert!(!d.is_blocked());
    assert_eq!(d.action, RuleAction::ForwardUpstream);
    assert!(d.reason.contains("whitelisted"));
}

#[test]
fn test_custom_rule_matching() {
    let mut rules = RulesEngine::builtin_rules();
    rules.push(Rule::new(
        "custom-tracker",
        "Custom Tracker",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "badtracker.xyz",
        RuleAction::Block403,
        150,
        "Custom rule",
    ));

    let engine = RulesEngine::new(rules, Vec::new());
    let d = engine.evaluate("https://api.badtracker.xyz/ping", "GET", None);
    assert!(d.is_blocked());
    assert_eq!(d.action, RuleAction::Block403);
}

#[test]
fn test_builtin_rules_intercept_browser_noise() {
    let engine = RulesEngine::with_builtin_rules();

    // 1. Chrome Widevine / Component updater
    let d1 = engine.evaluate(
        "https://edgedl.me.gvt1.com/edgedl/widevine/cdm.crx",
        "GET",
        None,
    );
    assert!(d1.is_blocked());
    assert_eq!(d1.action, RuleAction::Block204);

    // 2. Firefox OpenH264 binary codec download
    let d2 = engine.evaluate(
        "https://ciscobinary.openh264.org/openh264-win64-2.3.1.zip",
        "GET",
        None,
    );
    assert!(d2.is_blocked());
    assert_eq!(d2.action, RuleAction::Block204);

    // 3. Chrome update check
    let d3 = engine.evaluate(
        "https://update.googleapis.com/service/update2/json",
        "POST",
        None,
    );
    assert!(d3.is_blocked());

    // 4. Firefox AUS update check
    let d4 = engine.evaluate(
        "https://aus5.mozilla.org/update/6/Firefox/120.0.xml",
        "GET",
        None,
    );
    assert!(d4.is_blocked());

    // 5. ThreatMetrix profiling
    let d5 = engine.evaluate("https://h64.online-metrix.net/fp/tags.js", "GET", None);
    assert!(d5.is_blocked());

    // 6. LaunchDarkly telemetry
    let d6 = engine.evaluate(
        "https://events.launchdarkly.com/events/diagnostic",
        "POST",
        None,
    );
    assert!(d6.is_blocked());
}

#[test]
fn test_all_pattern_types_matching() {
    let exact = Rule::new(
        "exact-1",
        "Exact Match",
        RuleCategory::Custom,
        PatternType::ExactDomain,
        "api.example.com",
        RuleAction::Block403,
        100,
        "Exact domain only",
    );
    assert!(exact.matches("api.example.com", "https://api.example.com/data"));
    assert!(exact.matches("api.example.com:443", "https://api.example.com/data"));
    assert!(!exact.matches("sub.api.example.com", "https://sub.api.example.com/data"));

    let suffix = Rule::new(
        "suffix-1",
        "Suffix Match",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "example.com",
        RuleAction::Block403,
        100,
        "Domain suffix match",
    );
    assert!(suffix.matches("example.com", "https://example.com/"));
    assert!(suffix.matches("api.example.com", "https://api.example.com/"));
    assert!(!suffix.matches("badexample.com", "https://badexample.com/"));

    let url_contains = Rule::new(
        "url-1",
        "URL Contains",
        RuleCategory::Custom,
        PatternType::UrlContains,
        "/telemetry/v2",
        RuleAction::Block204,
        100,
        "Path snippet",
    );
    assert!(url_contains.matches("any.com", "https://any.com/api/telemetry/v2?id=1"));
    assert!(!url_contains.matches("any.com", "https://any.com/api/telemetry/v1?id=1"));

    let ext = Rule::new(
        "ext-1",
        "Extension Match",
        RuleCategory::Custom,
        PatternType::Extension,
        ".webm",
        RuleAction::Block204,
        100,
        "Video extension",
    );
    assert!(ext.matches("cdn.net", "https://cdn.net/video/sample.webm"));
    assert!(ext.matches("cdn.net", "https://cdn.net/video/sample.webm?token=abc"));
    assert!(!ext.matches("cdn.net", "https://cdn.net/video/sample.mp4"));
}

#[test]
fn test_disabled_rule_and_priority_order() {
    let mut disabled = Rule::new(
        "dis-1",
        "Disabled Rule",
        RuleCategory::Custom,
        PatternType::ExactDomain,
        "tracker.com",
        RuleAction::Block403,
        100,
        "Should not match",
    );
    disabled.enabled = false;
    assert!(!disabled.matches("tracker.com", "https://tracker.com/"));

    let r_low = Rule::new(
        "low",
        "Low Priority",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "priority-test.com",
        RuleAction::Block403,
        10,
        "Low priority",
    );
    let r_high = Rule::new(
        "high",
        "High Priority",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "priority-test.com",
        RuleAction::BypassDirect,
        100,
        "High priority",
    );

    let engine = RulesEngine::new(vec![r_low, r_high], Vec::new());
    let decision = engine.evaluate("https://api.priority-test.com/data", "GET", None);
    assert_eq!(decision.matched_rule_id.as_deref(), Some("high"));
    assert_eq!(decision.action, RuleAction::BypassDirect);
}

#[test]
fn test_resource_type_inference_and_sizes() {
    assert_eq!(
        ResourceType::infer("https://a.com/script.mjs", None),
        ResourceType::Script
    );
    assert_eq!(
        ResourceType::infer("https://a.com/style.css", None),
        ResourceType::Stylesheet
    );
    assert_eq!(
        ResourceType::infer("https://a.com/pic.webp", None),
        ResourceType::Image
    );
    assert_eq!(
        ResourceType::infer("https://a.com/font.ttf", None),
        ResourceType::Font
    );
    assert_eq!(
        ResourceType::infer("https://a.com/clip.mp3", None),
        ResourceType::MediaVideo
    );
    assert_eq!(
        ResourceType::infer("https://a.com/unknown", None),
        ResourceType::Other
    );

    // Explicit hints override extensions
    assert_eq!(
        ResourceType::infer("https://a.com/asset", Some("script")),
        ResourceType::Script
    );
    assert_eq!(
        ResourceType::infer("https://a.com/asset", Some("beacon")),
        ResourceType::Beacon
    );
    assert_eq!(
        ResourceType::infer("https://a.com/asset", Some("document")),
        ResourceType::Document
    );

    assert_eq!(ResourceType::MediaVideo.typical_byte_size(), 8_000_000);
    assert_eq!(ResourceType::Image.typical_byte_size(), 250_000);
    assert_eq!(ResourceType::Font.typical_byte_size(), 120_000);
    assert_eq!(ResourceType::Script.typical_byte_size(), 85_000);
    assert_eq!(ResourceType::Stylesheet.typical_byte_size(), 45_000);
    assert_eq!(ResourceType::Beacon.typical_byte_size(), 500);
}

#[test]
fn test_fbcdn_scontent_blocked_while_static_preserved() {
    let engine = RulesEngine::with_builtin_rules();

    // Heavy user media on scontent.*.fbcdn.net is blocked
    let scontent = engine.evaluate(
        "https://scontent.xx.fbcdn.net/v/t39.30808-6/banner.jpg",
        "GET",
        Some("image"),
    );
    assert!(scontent.is_blocked());

    // Core Meta JS/CSS on static.xx.fbcdn.net must NOT be blocked and uses BypassDirect split routing
    let static_js = engine.evaluate(
        "https://static.xx.fbcdn.net/rsrc.php/v3/core.js",
        "GET",
        Some("script"),
    );
    assert!(!static_js.is_blocked());
    assert_eq!(static_js.action, RuleAction::BypassDirect);

    // Stripe public JS SDK uses BypassDirect split routing while m.stripe.com stays ForwardUpstream
    let stripe_js = engine.evaluate("https://js.stripe.com/v3/", "GET", Some("script"));
    assert!(!stripe_js.is_blocked());
    assert_eq!(stripe_js.action, RuleAction::BypassDirect);

    let stripe_telemetry = engine.evaluate("https://m.stripe.com/6", "POST", Some("xhr"));
    assert_eq!(stripe_telemetry.action, RuleAction::ForwardUpstream);
}

#[test]
fn test_split_routing_precedence_reject_direct_and_named_pools() {
    let mut rules = RulesEngine::builtin_rules();
    // Wildcard suffix -> residential
    rules.push(Rule::new(
        "pool:suffix:sheerid",
        "SheerID Suffix",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "sheerid.com",
        RuleAction::RoutePool("residential".to_string()),
        130,
        "SheerID wildcard pool",
    ));
    // Exact domain -> higher priority (overrides suffix if different pool)
    rules.push(Rule::new(
        "pool:exact:services.sheerid.com",
        "SheerID Exact",
        RuleCategory::Custom,
        PatternType::ExactDomain,
        "services.sheerid.com",
        RuleAction::RoutePool("residential".to_string()),
        140,
        "SheerID exact pool",
    ));
    // Muse.ai -> static_isp_us
    rules.push(Rule::new(
        "pool:suffix:muse.ai",
        "Muse.ai Pool",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "muse.ai",
        RuleAction::RoutePool("static_isp_us".to_string()),
        130,
        "Muse.ai static US ISP",
    ));

    let engine = RulesEngine::with_default_route(rules, Vec::new(), DefaultRoute::Direct);

    // 1. Sensitive SheerID -> Pool("residential")
    let d_sheerid = engine.evaluate(
        "https://services.sheerid.com/rest/v2/verification",
        "POST",
        None,
    );
    assert_eq!(
        d_sheerid.action,
        RuleAction::RoutePool("residential".to_string())
    );
    assert_eq!(d_sheerid.target_pool(), Some("residential"));
    assert!(!d_sheerid.is_blocked());

    let d_sheerid_sub = engine.evaluate("https://my.sheerid.com/verify", "GET", None);
    assert_eq!(
        d_sheerid_sub.action,
        RuleAction::RoutePool("residential".to_string())
    );

    // 2. Muse.ai & auth subdomain -> Pool("static_isp_us")
    let d_muse = engine.evaluate("https://auth.muse.ai/oauth/login", "POST", None);
    assert_eq!(
        d_muse.action,
        RuleAction::RoutePool("static_isp_us".to_string())
    );

    // 3. Built-in tracker/font -> Reject
    let d_gtm = engine.evaluate(
        "https://www.googletagmanager.com/gtm.js",
        "GET",
        Some("script"),
    );
    assert!(d_gtm.is_blocked());

    // 4. Unmatched public domain -> DefaultRoute::Direct
    let d_other = engine.evaluate(
        "https://cdn.example-static.org/app.js",
        "GET",
        Some("script"),
    );
    assert_eq!(d_other.action, RuleAction::BypassDirect);
}
