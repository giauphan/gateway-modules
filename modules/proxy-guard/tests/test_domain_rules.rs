use proxy_guard::domain::{PatternType, ResourceType, Rule, RuleAction, RuleCategory};
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
