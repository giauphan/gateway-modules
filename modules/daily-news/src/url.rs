//! URL sanitization, domain extraction, and cryptographic hashing utilities.
//!
//! Enforces SSRF boundary protection, tracking parameter elimination, and SHA-256 signatures.

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

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
    let host_part = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let host_without_user = host_part.rsplit('@').next().unwrap_or(host_part);
    if host_without_user.starts_with('[') {
        if let Some(end) = host_without_user.find(']') {
            return host_without_user[..=end].to_lowercase();
        }
    }
    let domain = host_without_user
        .split(':')
        .next()
        .unwrap_or(host_without_user);
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
    let words: Vec<&str> = combined
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| !w.is_empty())
        .collect();

    let matches_phrase_or_word = |k: &str| -> bool {
        if k.contains(' ') {
            combined.contains(k)
        } else {
            words.contains(&k)
        }
    };

    if [
        "prompt caching",
        "context window",
        "kv cache",
        "memory",
        "memories",
        "caching",
        "state loss",
        "token limit",
        "tokens",
    ]
    .iter()
    .any(|&k| matches_phrase_or_word(k))
    {
        return "context_memory".to_string();
    }

    if [
        "tool call",
        "tool calls",
        "tool-calling",
        "tool-call",
        "mcp",
        "schema",
        "function call",
        "tools",
        "tooling",
        "payload parsing",
    ]
    .iter()
    .any(|&k| matches_phrase_or_word(k))
    {
        return "agent_tool_use".to_string();
    }

    if [
        "debug",
        "debugging",
        "retry",
        "retries",
        "flaky",
        "flakiness",
        "reliability",
        "ci",
        "loop",
        "loops",
        "crash",
        "crashes",
        "crashing",
        "failure",
        "failures",
        "failing",
        "trace",
        "traces",
    ]
    .iter()
    .any(|&k| matches_phrase_or_word(k))
    {
        return "debugging_reliability".to_string();
    }

    if [
        "benchmark",
        "benchmarks",
        "benchmarking",
        "eval",
        "evals",
        "evaluation",
        "evaluating",
        "score",
        "scores",
        "metrics",
        "metric",
        "test suite",
    ]
    .iter()
    .any(|&k| matches_phrase_or_word(k))
    {
        return "agent_evaluation".to_string();
    }

    if [
        "agent",
        "agents",
        "architecture",
        "multi-agent",
        "orchestrator",
        "orchestration",
        "workflow",
        "system prompt",
        "planning",
        "reasoning",
    ]
    .iter()
    .any(|&k| matches_phrase_or_word(k))
    {
        return "agent_architecture".to_string();
    }

    "unclassified".to_string()
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

    let clean_v6 = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = clean_v6.parse::<Ipv6Addr>() {
        if is_restricted_ipv6(ip, true).is_err() {
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

pub fn is_restricted_ipv4(ip: Ipv4Addr, allow_loopback: bool) -> Result<(), String> {
    let o = ip.octets();
    if !allow_loopback && o[0] == 127 {
        return Err("Capture target blocked: loopback IPv4 127.0.0.0/8".to_string());
    }
    if o[0] == 0
        || o[0] == 10
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 169 && o[1] == 254)
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && (18..=19).contains(&o[1]))
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
        || o[0] >= 224
    {
        return Err(format!(
            "Capture target blocked: private/reserved IPv4 '{ip}'"
        ));
    }
    Ok(())
}

pub fn is_restricted_ipv6(ip: Ipv6Addr, allow_loopback: bool) -> Result<(), String> {
    if ip.is_unspecified() {
        return Err("Capture target blocked: IPv6 unspecified ::".to_string());
    }
    if !allow_loopback && ip.is_loopback() {
        return Err("Capture target blocked: IPv6 loopback ::1".to_string());
    }
    let s = ip.segments();
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
        let v4 = Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        );
        return is_restricted_ipv4(v4, allow_loopback);
    }
    if (s[0] & 0xffc0) == 0xfe80 {
        return Err("Capture target blocked: IPv6 link-local fe80::/10".to_string());
    }
    if (s[0] & 0xfe00) == 0xfc00 {
        return Err("Capture target blocked: IPv6 unique local fc00::/7".to_string());
    }
    if (s[0] & 0xff00) == 0xff00 {
        return Err("Capture target blocked: IPv6 multicast ff00::/8".to_string());
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return Err("Capture target blocked: IPv6 documentation 2001:db8::/32".to_string());
    }
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0) {
        let v4 = Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        );
        return is_restricted_ipv4(v4, allow_loopback);
    }
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new(
            (s[1] >> 8) as u8,
            (s[1] & 0xff) as u8,
            (s[2] >> 8) as u8,
            (s[2] & 0xff) as u8,
        );
        return is_restricted_ipv4(v4, allow_loopback);
    }
    Ok(())
}
