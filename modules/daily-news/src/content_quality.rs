//! Content quality gate and anti-bot challenge detection for captured articles.
//!
//! Enforces Clean Architecture domain rules:
//! Discards bot challenge interstitials, cookie consent walls, paywall placeholders,
//! and empty navigation shells before marking source capture as valid.

pub const INTERSTITIAL_TITLES: &[&str] = &[
    "attention required! | cloudflare",
    "just a moment...",
    "checking your browser",
    "ddos-guard",
    "security check",
    "access denied",
    "403 forbidden",
];

pub const VISIBLE_CHALLENGE_PATTERNS: &[&str] = &[
    "verify you are human",
    "checking your browser before accessing",
    "checking your browser",
    "please enable javascript and cookies",
    "security service to protect itself from online attacks",
    "please complete the security check",
    "ray id:",
    "human verification",
];

pub const PAYWALL_PATTERNS: &[&str] = &[
    "to continue reading, subscribe",
    "this article is for subscribers only",
    "create a free account to continue reading",
    "you have reached your limit of free articles",
    "subscribe now for unlimited access",
    "become a member to read the full story",
    "sign in to your subscription",
    "sign in to continue reading",
    "log in to continue reading",
    "register to continue reading",
    "sign up to continue reading",
    "sign in to read the full article",
];

pub const NAV_ONLY_PATTERNS: &[&str] = &[
    "enable javascript to run this app",
    "javascript is required",
    "you need to enable javascript",
    "cookie policy",
    "accept all cookies",
];

pub const NAV_MENU_TOKENS: &[&str] = &[
    "home", "about", "services", "products", "contact", "careers", "pricing", "blog",
    "faq", "terms", "privacy", "login", "sign in", "navigation",
];

pub const MIN_ARTICLE_WORD_COUNT: usize = 5;
pub const MIN_ARTICLE_CHAR_COUNT: usize = 30;

pub fn evaluate_content_quality(
    page_title: Option<&str>,
    html: &str,
    main_text: &str,
) -> Result<String, String> {
    if let Some(title) = page_title {
        let t_trimmed = title.trim().to_lowercase();
        for &pattern in INTERSTITIAL_TITLES {
            if t_trimmed == pattern {
                return Err(format!("anti_bot_challenge_detected: page title matched '{pattern}'"));
            }
        }
    }

    let text_lower = main_text.to_lowercase();
    let html_lower = html.to_lowercase();
    let words = main_text.split_whitespace().count();
    let chars = main_text.trim().chars().count();

    for &pw in PAYWALL_PATTERNS {
        if text_lower.contains(pw) {
            return Err(format!("paywall_or_placeholder_detected: content matched '{pw}'"));
        }
    }

    for &pattern in VISIBLE_CHALLENGE_PATTERNS {
        if text_lower.contains(pattern) {
            return Err(format!("anti_bot_challenge_detected: content matched '{pattern}'"));
        }
    }
    if html_lower.contains("cf-browser-verification") {
        return Err("anti_bot_challenge_detected: content matched 'cf-browser-verification'".to_string());
    }

    if words < MIN_ARTICLE_WORD_COUNT || chars < MIN_ARTICLE_CHAR_COUNT {
        return Err(format!(
            "insufficient_article_content: body has only {words} words / {chars} chars (minimum required: {MIN_ARTICLE_WORD_COUNT} words / {MIN_ARTICLE_CHAR_COUNT} chars)"
        ));
    }

    if words < 100 {
        for &nav in NAV_ONLY_PATTERNS {
            if text_lower.contains(nav) && (words < 40 || text_lower.len() < 250) {
                return Err(format!("nav_or_cookie_only: content matched '{nav}'"));
            }
        }
        let clean_tokens: Vec<&str> = text_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        let nav_count = clean_tokens.iter().filter(|w| NAV_MENU_TOKENS.contains(w)).count();
        let has_sentence = main_text.chars().any(|c| c == '.' || c == '!' || c == '?')
            && clean_tokens.iter().any(|w| matches!(*w, "the" | "is" | "are" | "in" | "to" | "for" | "with" | "and" | "on" | "by" | "a" | "of" | "from" | "we" | "our" | "demonstrated" | "found" | "shows"));

        if !has_sentence || (nav_count > 0 && nav_count * 2 >= clean_tokens.len()) || nav_count >= 5 {
            return Err("nav_or_cookie_only: content appears to be a navigation menu or empty shell".to_string());
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
