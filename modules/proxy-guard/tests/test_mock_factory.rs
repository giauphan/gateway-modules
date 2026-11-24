//! Tests for MockFactory synthetic mock generation.

use proxy_guard::domain::RuleAction;
use proxy_guard::mock_factory::MockFactory;

#[test]
fn test_mock_factory_script_stubs() {
    // 1. Google Tag Manager & Google Analytics
    let gtm = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://www.googletagmanager.com/gtm.js",
    )
    .expect("GTM mock must be created");
    assert_eq!(gtm.status, 200);
    assert_eq!(gtm.content_type, "application/javascript; charset=utf-8");
    assert!(gtm.body.contains("dataLayer"));
    assert!(gtm.body.contains("gtag"));

    let ga = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://www.google-analytics.com/analytics.js",
    )
    .expect("GA mock must be created");
    assert_eq!(ga.status, 200);
    assert!(ga.body.contains("dataLayer"));

    // 2. Facebook Pixel
    let fb = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://connect.facebook.net/en_US/fbevents.js",
    )
    .expect("FB mock must be created");
    assert_eq!(fb.status, 200);
    assert!(fb.body.contains("window.fbq"));

    // 3. TikTok Pixel
    let tt = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://analytics.tiktok.com/i18n/pixel/sdk.js",
    )
    .expect("TikTok mock must be created");
    assert_eq!(tt.status, 200);
    assert!(tt.body.contains("window.ttq"));

    // 4. Sentry & Datadog Error Telemetry
    let sentry = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://browser.sentry-cdn.com/bundle.min.js",
    )
    .expect("Sentry mock must be created");
    assert_eq!(sentry.status, 200);
    assert!(sentry.body.contains("window.Sentry"));

    let datadog = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://browser-intake-datadoghq.com/rum.js",
    )
    .expect("Datadog mock must be created");
    assert_eq!(datadog.status, 200);
    assert!(datadog.body.contains("DD_RUM"));

    // 5. Generic Mock Script
    let generic = MockFactory::create_mock(
        &RuleAction::MockScript,
        "https://custom-analytics.example.com/tracker.js",
    )
    .expect("Generic mock must be created");
    assert_eq!(generic.status, 200);
    assert!(generic.body.contains("Blocked Script Stub"));
}

#[test]
fn test_mock_factory_pixel_and_binary() {
    let pixel = MockFactory::create_mock(&RuleAction::MockPixel, "https://example.com/pixel.gif")
        .expect("Pixel mock must be created");
    assert_eq!(pixel.status, 200);
    assert_eq!(pixel.content_type, "image/gif");
    assert_eq!(pixel.body, MockFactory::TRANSPARENT_1X1_GIF);

    // Raw binary constant check
    assert_eq!(MockFactory::TRANSPARENT_GIF_BYTES[0], b'G');
    assert_eq!(MockFactory::TRANSPARENT_GIF_BYTES[1], b'I');
    assert_eq!(MockFactory::TRANSPARENT_GIF_BYTES[2], b'F');
}

#[test]
fn test_mock_factory_empty_json_and_204() {
    let empty_json = MockFactory::create_mock(
        &RuleAction::BlockEmptyJson,
        "https://api.segment.io/v1/track",
    )
    .expect("Empty JSON mock must be created");
    assert_eq!(empty_json.status, 200);
    assert_eq!(empty_json.content_type, "application/json");
    assert_eq!(empty_json.body, "{}");

    let no_content =
        MockFactory::create_mock(&RuleAction::Block204, "https://example.com/beacon.gif")
            .expect("204 mock must be created");
    assert_eq!(no_content.status, 204);
    assert!(no_content.body.is_empty());

    let forbidden = MockFactory::create_mock(&RuleAction::Block403, "https://badsite.com/track")
        .expect("403 mock must be created");
    assert_eq!(forbidden.status, 403);
    assert!(forbidden.body.contains("Blocked by Proxy Guard"));

    // Forward and Bypass must not synthesize mocks
    assert!(
        MockFactory::create_mock(&RuleAction::ForwardUpstream, "https://example.com").is_none()
    );
    assert!(MockFactory::create_mock(&RuleAction::BypassDirect, "https://example.com").is_none());
}
