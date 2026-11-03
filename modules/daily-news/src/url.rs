//! URL sanitization, domain extraction, and cryptographic hashing utilities.
//!
//! Enforces SSRF boundary protection, tracking parameter elimination, and SHA-256 signatures.

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::net::Ipv4Addr;

pub const DEFAULT_ALLOWED_DOMAINS: &[&str] = &[
    "docs.anthropic.com",
    "docs.github.com",
    "modelcontextprotocol.io",
    "microsoft.github.io",
    "openai.com",
    "arxiv.org",
    "github.com",
    "git-scm.com",
    "news.ycombinator.com",
    "producthunt.com",
    "techcrunch.com",
    "theverge.com",
    "wired.com",
    "arstechnica.com",
    "venturebeat.com",
];

pub const TRACKING_QUERY_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "fbclid",
    "gclid",
    "msclkid",
    "ref",
    "source",
    "rss",
];

pub fn clean_html_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    let mut tag_buf = String::new();
    for ch in raw.chars() {
        if ch == '<' {
            in_tag = true;
            tag_buf.clear();
        } else if ch == '>' && in_tag {
            in_tag = false;
            let tag_lower = tag_buf.trim().to_ascii_lowercase();
            if tag_lower == "p" || tag_lower == "/p" || tag_lower.starts_with("br") {
                out.push(' ');
            }
        } else if in_tag {
            tag_buf.push(ch);
        } else {
            out.push(ch);
        }
    }
    let decoded = out
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
        .replace("&nbsp;", " ");
    decoded.split_whitespace().collect::<Vec<&str>>().join(" ")
}

pub fn extract_hn_item_id(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("hn_") {
        return rest.parse::<u64>().ok();
    }
    if let Some(rest) = trimmed.strip_prefix("newsnow_hackernews_") {
        return rest.parse::<u64>().ok();
    }
    if trimmed.contains("news.ycombinator.com/item") {
        if let Some((_, query)) = trimmed.split_once("id=") {
            let digits: String = query.chars().take_while(|c| c.is_ascii_digit()).collect();
            return digits.parse::<u64>().ok();
        }
    }
    None
}

pub fn extract_domain(url: &str) -> String {
    let trimmed = url.trim();
    let without_scheme = if let Some(stripped) = trimmed.strip_prefix("https://") {
        stripped
    } else if let Some(stripped) = trimmed.strip_prefix("http://") {
        stripped
    } else {
        trimmed
    };
    let host_part = without_scheme.split('/').next().unwrap_or(without_scheme);
    let domain = host_part.split(':').next().unwrap_or(host_part);
    domain.trim_start_matches("www.").to_lowercase()
}

pub fn normalize_publisher(publisher: Option<&str>) -> String {
    match publisher {
        None => "Unknown".to_string(),
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return "Unknown".to_string();
            }
            let mut cleaned = trimmed.to_string();
            for suffix in &[".com", ".org", ".net", ".co", ".io", ".gov", ".news", ".sh"] {
                if cleaned.to_lowercase().ends_with(suffix) {
                    cleaned.truncate(cleaned.len() - suffix.len());
                    break;
                }
            }
            let out = cleaned.trim();
            if out.is_empty() {
                "Unknown".to_string()
            } else {
                out.to_string()
            }
        }
    }
}

pub fn canonicalize_url(url: &str) -> String {
    let clean = url.trim();
    if clean.is_empty() {
        return String::new();
    }
    let without_fragment = clean.split('#').next().unwrap_or(clean);
    if let Some((base, query)) = without_fragment.split_once('?') {
        let filtered: Vec<&str> = query
            .split('&')
            .filter(|p| {
                let key = p.split('=').next().unwrap_or("").to_lowercase();
                !TRACKING_QUERY_PARAMS
                    .iter()
                    .any(|&tp| key == tp || key.starts_with("utm_"))
            })
            .collect();
        if filtered.is_empty() {
            base.trim_end_matches('/').to_string()
        } else {
            format!("{}?{}", base.trim_end_matches('/'), filtered.join("&"))
        }
    } else {
        without_fragment.trim_end_matches('/').to_string()
    }
}

pub fn compute_sha256(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in result {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub fn compute_title_hash(title: &str) -> String {
    let norm: String = title
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();
    compute_sha256(&norm)
}

pub fn compute_content_hash(title: &str, body: Option<&str>, canonical_url: &str) -> String {
    let clean_title: String = title
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();
    let clean_url = canonical_url.trim().to_lowercase();
    let body_str = body.unwrap_or("").trim();
    let limit = body_str.chars().take(500).collect::<String>();
    let clean_body: String = limit
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();
    let payload = format!("{clean_title}|{clean_url}|{clean_body}");
    compute_sha256(&payload)
}

pub fn classify_topic_family(title: &str, text: &str) -> String {
    let combined = format!("{title} {text}").to_lowercase();
    if [
        "prompt caching",
        "context window",
        "kv cache",
        "memory",
        "state loss",
        "token limit",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "context_memory".to_string();
    }
    if [
        "tool call",
        "mcp",
        "schema",
        "function call",
        "tools",
        "payload parsing",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "agent_tool_use".to_string();
    }
    if [
        "debug",
        "retry",
        "flaky",
        "reliability",
        "ci",
        "loop",
        "crash",
        "failure",
        "trace",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "debugging_reliability".to_string();
    }
    if [
        "benchmark",
        "eval",
        "evaluation",
        "score",
        "metrics",
        "test suite",
    ]
    .iter()
    .any(|k| combined.contains(k))
    {
        return "agent_evaluation".to_string();
    }
    "agent_architecture".to_string()
}

pub fn is_safe_url(url: &str, allowed_domains: Option<&HashSet<String>>) -> bool {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return false;
    }
    let host = extract_domain(trimmed);
    if host.is_empty() {
        return false;
    }

    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let octets = ip.octets();
        if octets[0] == 127 {
            return true;
        }
        if octets[0] == 10
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            || (octets[0] == 192 && octets[1] == 168)
            || (octets[0] == 169 && octets[1] == 254)
            || octets[0] == 0
        {
            return false;
        }
    }

    if let Some(allowed) = allowed_domains {
        return allowed
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")));
    }
    true
}
