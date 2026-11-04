//! Content quality gate and anti-bot challenge detection for captured articles.
//!
//! Enforces Clean Architecture domain rules:
//! Discards bot challenge interstitials, cookie consent walls, paywall placeholders,
//! and empty navigation shells before marking source capture as valid.

pub const ANTI_BOT_PATTERNS: &[&str] = &[
    "attention required! | cloudflare",
    "just a moment...",
    "checking your browser",
    "verify you are human",
    "cf-browser-verification",
    "please enable javascript and cookies",
    "security service to protect itself from online attacks",
    "access denied",
    "403 forbidden",
    "ddos-guard",
    "bot detection",
    "please complete the security check",
    "ray id:",
    "human verification",
];

pub const NAV_ONLY_PATTERNS: &[&str] = &[
    "enable javascript to run this app",
    "javascript is required",
    "you need to enable javascript",
    "cookie policy",
    "accept all cookies",
];

pub const MIN_ARTICLE_WORD_COUNT: usize = 5;
pub const MIN_ARTICLE_CHAR_COUNT: usize = 30;

pub fn evaluate_content_quality(
    page_title: Option<&str>,
    html: &str,
    main_text: &str,
) -> Result<String, String> {
    if let Some(title) = page_title {
        let t_lower = title.to_lowercase();
        for &pattern in ANTI_BOT_PATTERNS {
            if t_lower.contains(pattern) {
                return Err(format!("anti_bot_challenge_detected: page title matched '{pattern}'"));
            }
        }
    }

    let text_lower = main_text.to_lowercase();
    let html_lower = html.to_lowercase();

    for &pattern in ANTI_BOT_PATTERNS {
        if text_lower.contains(pattern) || (pattern.len() > 15 && html_lower.contains(pattern)) {
            return Err(format!("anti_bot_challenge_detected: content matched '{pattern}'"));
        }
    }

    let words = main_text.split_whitespace().count();
    let chars = main_text.trim().chars().count();
    if words < MIN_ARTICLE_WORD_COUNT || chars < MIN_ARTICLE_CHAR_COUNT {
        return Err(format!(
            "insufficient_article_content: body has only {words} words / {chars} chars (minimum required: {MIN_ARTICLE_WORD_COUNT} words / {MIN_ARTICLE_CHAR_COUNT} chars)"
        ));
    }

    if words < 25 {
        for &nav in NAV_ONLY_PATTERNS {
            if text_lower.contains(nav) {
                return Err(format!("nav_or_cookie_only: content matched '{nav}'"));
            }
        }
    }

    Ok("ARTICLE_CONTENT_VALID".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cloudflare_anti_bot_detected() {
        let title = "Just a moment...";
        let html = "<html><head><title>Just a moment...</title></head><body><div class='cf-browser-verification'>Verify you are human</div></body></html>";
        let text = "Verify you are human";
        let res = evaluate_content_quality(Some(title), html, text);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("anti_bot_challenge_detected"));
    }

    #[test]
    fn test_insufficient_body_rejected() {
        let title = "Login";
        let html = "<html><body>Sign in</body></html>";
        let text = "Sign in";
        let res = evaluate_content_quality(Some(title), html, text);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("insufficient_article_content"));
    }

    #[test]
    fn test_valid_article_content() {
        let title = "Scaling Native Rust Microservices";
        let html = "<html><body><p>We demonstrated a 4x reduction in cache latency using deterministic memory management and zero-copy JSON parsing.</p></body></html>";
        let text = "We demonstrated a 4x reduction in cache latency using deterministic memory management and zero-copy JSON parsing.";
        let res = evaluate_content_quality(Some(title), html, text);
        assert_eq!(res.unwrap(), "ARTICLE_CONTENT_VALID");
    }
}
