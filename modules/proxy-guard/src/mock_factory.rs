//! Synthetic mock generators for blocked assets.
//!
//! Clean Architecture: Domain Helper / Value Generator.
//! Ensures client scripts and automated browser flows do not crash
//! when analytics or tracker requests are intercepted.

use crate::domain::{MockPayload, RuleAction};

pub struct MockFactory;

impl MockFactory {
    /// 1x1 transparent GIF89a raw binary bytes (standard 42-byte RFC representation).
    pub const TRANSPARENT_GIF_BYTES: &[u8] = &[
        71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 0, 0, 0, 255, 255, 255, 33, 249, 4, 1, 0, 0,
        0, 0, 44, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 1, 68, 0, 59,
    ];

    /// 1x1 transparent GIF89a base64 string.
    pub const TRANSPARENT_1X1_GIF: &str =
        "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

    /// Universal Google Analytics & Tag Manager harmless stub script.
    pub const GA_GTM_STUB_JS: &str = r#"
/* [Proxy Guard] GA/GTM Stubbed for zero proxy bandwidth */
window.dataLayer = window.dataLayer || [];
window.gtag = window.gtag || function() { (window.dataLayer.push(arguments)); };
window.ga = window.ga || function(cb) { if (typeof cb === 'function') { cb(); } };
window.google_tag_manager = window.google_tag_manager || {};
"#;

    /// Meta / Facebook Pixel harmless stub script.
    pub const FB_PIXEL_STUB_JS: &str = r#"
/* [Proxy Guard] Meta Pixel Stubbed */
window.fbq = window.fbq || function() {};
window._fbq = window.fbq;
"#;

    /// TikTok Pixel harmless stub script.
    pub const TIKTOK_PIXEL_STUB_JS: &str = r#"
/* [Proxy Guard] TikTok Pixel Stubbed */
window.ttq = window.ttq || { track: function(){}, page: function(){}, identify: function(){} };
"#;

    /// Generic error tracker harmless stub (Sentry, Datadog RUM).
    pub const ERROR_TRACKER_STUB_JS: &str = r#"
/* [Proxy Guard] Telemetry & Sentry Stubbed */
window.Sentry = window.Sentry || { captureException: function(){}, captureMessage: function(){}, init: function(){} };
window.DD_RUM = window.DD_RUM || { init: function(){}, addAction: function(){} };
"#;

    /// Synthesizes a mock response for a given action and URL.
    pub fn create_mock(action: &RuleAction, url: &str) -> Option<MockPayload> {
        match action {
            RuleAction::MockScript => {
                let lower = url.to_ascii_lowercase();
                let body =
                    if lower.contains("googletagmanager") || lower.contains("google-analytics") {
                        Self::GA_GTM_STUB_JS
                    } else if lower.contains("facebook.net") || lower.contains("fbq") {
                        Self::FB_PIXEL_STUB_JS
                    } else if lower.contains("tiktok") {
                        Self::TIKTOK_PIXEL_STUB_JS
                    } else if lower.contains("sentry") || lower.contains("datadog") {
                        Self::ERROR_TRACKER_STUB_JS
                    } else {
                        "/* [Proxy Guard] Blocked Script Stub */\n"
                    };

                Some(MockPayload {
                    status: 200,
                    content_type: "application/javascript; charset=utf-8".to_string(),
                    body: body.trim().to_string(),
                })
            }
            RuleAction::MockPixel => Some(MockPayload {
                status: 200,
                content_type: "image/gif".to_string(),
                body: Self::TRANSPARENT_1X1_GIF.to_string(),
            }),
            RuleAction::Block204 => Some(MockPayload {
                status: 204,
                content_type: "text/plain".to_string(),
                body: String::new(),
            }),
            RuleAction::BlockEmptyJson => Some(MockPayload {
                status: 200,
                content_type: "application/json".to_string(),
                body: "{}".to_string(),
            }),
            RuleAction::Block403 => Some(MockPayload {
                status: 403,
                content_type: "text/plain".to_string(),
                body: "Blocked by Proxy Guard".to_string(),
            }),
            RuleAction::ForwardUpstream | RuleAction::BypassDirect => None,
        }
    }
}
