//! Domain entities and value objects for upstream proxies, routing strategies, and authentication.
//!
//! Clean Architecture: Core Domain Layer.
//! Pure business rules with zero transport, framework, or database dependencies.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Upstream proxy protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Http,
    Https,
    Socks5,
}

impl ProxyProtocol {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "socks5" | "socks5h" => Self::Socks5,
            "https" => Self::Https,
            _ => Self::Http,
        }
    }

    pub fn scheme(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
        }
    }
}

/// Upstream proxy authentication credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

/// Upstream proxy configuration and health metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamProxy {
    pub id: String,
    pub name: String,
    pub url: String,
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: u16,
    pub auth: Option<ProxyAuth>,
    pub country: Option<String>,
    pub weight: u32,
    pub enabled: bool,
    pub failure_count: u32,
    pub success_count: u64,
    pub tags: Vec<String>,
}

impl UpstreamProxy {
    /// Parses an upstream proxy definition string into a structured UpstreamProxy entity.
    ///
    /// Supported formats:
    /// - `http://user:pass@host:port` (optional `#US` or `|US` country acronym suffix, or `US|` prefix)
    /// - `socks5://host:port`
    /// - `host:port:user:pass` or `host:port:user:pass:US` (standard proxy vendor format)
    /// - `host:port`
    pub fn parse(
        raw_url: &str,
        name: Option<&str>,
        country: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Self, String> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            return Err("Proxy URL cannot be empty".to_string());
        }

        let (stripped_url, inline_tag_country, inline_tag) = split_inline_country_and_tag(trimmed);
        let (protocol, host, port, auth, vendor_country) = parse_proxy_components(stripped_url)?;

        let id = match &auth {
            Some(a) => format!("{}:{}:{}", host, port, a.username),
            None => format!("{}:{}", host, port),
        };
        let proxy_name = name.unwrap_or(&id).to_string();

        let canonical_url = match &auth {
            Some(a) => format!(
                "{}://{}:{}@{}:{}",
                protocol.scheme(),
                url_encode_userinfo(&a.username),
                url_encode_userinfo(&a.password),
                host,
                port
            ),
            None => format!("{}://{}:{}", protocol.scheme(), host, port),
        };

        let parsed_country_arg = country.and_then(parse_country_input);
        let (explicit_country, force_cc_username) = match parsed_country_arg {
            Some((cc, append_cc)) => (Some(cc), append_cc),
            None => (None, false),
        };

        let detected_country = explicit_country
            .or(inline_tag_country)
            .or(vendor_country)
            .or_else(|| {
                auth.as_ref()
                    .and_then(|a| extract_country_from_username(&a.username))
            });

        let mut merged_tags = tags.unwrap_or_default();
        if let Some(t) = inline_tag {
            if !merged_tags.iter().any(|existing| existing.eq_ignore_ascii_case(&t)) {
                merged_tags.push(t);
            }
        }
        if let Some(tier_tag) = infer_proxy_tier_tag(&host, auth.as_ref().map(|a| a.username.as_str())) {
            if !merged_tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(tier_tag))
            {
                merged_tags.push(tier_tag.to_string());
            }
        }

        let proxy = Self {
            id,
            name: proxy_name,
            url: canonical_url,
            protocol,
            host,
            port,
            auth,
            country: detected_country.clone(),
            weight: 1,
            enabled: true,
            failure_count: 0,
            success_count: 0,
            tags: merged_tags,
        };

        if force_cc_username {
            if let Some(ref cc) = detected_country {
                return Ok(proxy.with_cc_username(cc));
            }
        }

        Ok(proxy)
    }

    /// Returns the canonical proxy URL with passwords masked (safe for logging & MCP outputs).
    pub fn masked_url(&self) -> String {
        match &self.auth {
            Some(a) => format!(
                "{}://{}:***@{}:{}",
                self.protocol.scheme(),
                url_encode_userinfo(&a.username),
                self.host,
                self.port
            ),
            None => format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port),
        }
    }

    /// Returns a copy of the proxy entity with credentials masked for safe serialization.
    pub fn to_masked(&self) -> Self {
        let mut cloned = self.clone();
        cloned.url = self.masked_url();
        if let Some(ref mut a) = cloned.auth {
            a.password = "***".to_string();
        }
        cloned
    }

    /// Returns a 1-line pool representation, preserving an explicit `#CC` country suffix
    /// when the country acronym is not already encoded in the username.
    pub fn pool_line(&self) -> String {
        if let Some(ref cc) = self.country {
            let in_username = self
                .auth
                .as_ref()
                .and_then(|a| extract_country_from_username(&a.username))
                .map(|u| u.eq_ignore_ascii_case(cc))
                .unwrap_or(false);
            if !in_username {
                return format!("{}#{}", self.url, cc);
            }
        }
        self.url.clone()
    }

    /// Formats this proxy into a standard format for client tools (Playwright, Puppeteer).
    pub fn to_playwright_config(&self) -> PlaywrightProxyConfig {
        PlaywrightProxyConfig {
            server: format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port),
            username: self.auth.as_ref().map(|a| a.username.clone()),
            password: self.auth.as_ref().map(|a| a.password.clone()),
        }
    }

    /// Returns a copy of this proxy with `-cc-<country>` appended/replaced in its username
    /// (and updates `id`, `url`, and `country` accordingly).
    pub fn with_cc_username(&self, target_country: &str) -> Self {
        let target_upper = target_country.trim().to_ascii_uppercase();
        let target_lower = target_upper.to_ascii_lowercase();
        let mut updated = self.clone();
        updated.country = Some(target_upper);
        if let Some(ref a) = self.auth {
            let new_username = replace_country_in_username(&a.username, &target_lower)
                .unwrap_or_else(|| format!("{}-cc-{}", a.username, target_lower));
            let new_id = format!("{}:{}:{}", updated.host, updated.port, new_username);
            updated.id = new_id.clone();
            if updated.name == self.id {
                updated.name = new_id;
            }
            let new_auth = ProxyAuth {
                username: new_username.clone(),
                password: a.password.clone(),
            };
            updated.url = format!(
                "{}://{}:{}@{}:{}",
                updated.protocol.scheme(),
                url_encode_userinfo(&new_username),
                url_encode_userinfo(&new_auth.password),
                updated.host,
                updated.port
            );
            updated.auth = Some(new_auth);
        }
        updated
    }

    /// Injects a deterministic, provider-safe sticky session token (`-sessid-<hex>-sesstime-10`)
    /// into residential proxy usernames so all requests sharing `session_id` exit from the same
    /// clean residential IP instead of rotating mid-session.
    pub fn with_sticky_session(&self, session_id: &str) -> Self {
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            return self.clone();
        }
        let a = match self.auth.as_ref() {
            Some(auth) => auth,
            None => return self.clone(),
        };
        let host_lower = self.host.to_ascii_lowercase();
        let user_lower = a.username.to_ascii_lowercase();
        let is_residential = host_lower.starts_with("pr.")
            || user_lower.starts_with("customer-")
            || user_lower.contains("resi")
            || user_lower.contains("-sessid-");
        if !is_residential {
            return self.clone();
        }

        // Strip any existing -sessid-... suffix before appending the new clean token
        let base_username = if let Some(idx) = user_lower.find("-sessid-") {
            &a.username[..idx]
        } else {
            a.username.as_str()
        };

        // Oxylabs/BrightData require strictly alphanumeric sessid tokens (no `@`, `.`, or `-`)
        let digest = Sha256::digest(trimmed.as_bytes());
        let mut token = String::with_capacity(12);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for &b in &digest[..6] {
            token.push(HEX[(b >> 4) as usize] as char);
            token.push(HEX[(b & 0x0f) as usize] as char);
        }

        let new_username = format!("{base_username}-sessid-{token}-sesstime-10");
        let mut updated = self.clone();
        let new_auth = ProxyAuth {
            username: new_username.clone(),
            password: a.password.clone(),
        };
        updated.url = format!(
            "{}://{}:{}@{}:{}",
            updated.protocol.scheme(),
            url_encode_userinfo(&new_username),
            url_encode_userinfo(&new_auth.password),
            updated.host,
            updated.port
        );
        updated.auth = Some(new_auth);
        updated
    }

    /// Dynamically adapts a residential proxy to target another country if supported by its auth scheme.
    /// E.g., for Oxylabs/BrightData residential pools (`-cc-vn` -> `-cc-us`).
    pub fn adapt_country(&self, target_country: &str) -> Option<Self> {
        let a = self.auth.as_ref()?;
        let new_username = replace_country_in_username(&a.username, target_country)?;
        let mut adapted = self.clone();
        let target_upper = target_country.trim().to_ascii_uppercase();
        adapted.country = Some(target_upper);
        let new_id = format!("{}:{}:{}", adapted.host, adapted.port, new_username);
        adapted.id = new_id.clone();
        adapted.name = new_id;
        let new_auth = ProxyAuth {
            username: new_username.clone(),
            password: a.password.clone(),
        };
        adapted.url = format!(
            "{}://{}:{}@{}:{}",
            adapted.protocol.scheme(),
            url_encode_userinfo(&new_username),
            url_encode_userinfo(&new_auth.password),
            adapted.host,
            adapted.port
        );
        adapted.auth = Some(new_auth);
        Some(adapted)
    }
}

fn infer_proxy_tier_tag(host: &str, username: Option<&str>) -> Option<&'static str> {
    let h = host.to_ascii_lowercase();
    let u = username.unwrap_or("").to_ascii_lowercase();
    if h.starts_with("dc.") || u.starts_with("user-dc") || u.contains("-dc_") || u.contains("_dc_")
    {
        Some("datacenter")
    } else if h.starts_with("pr.") || u.starts_with("customer-") || u.contains("resi") {
        Some("residential")
    } else {
        None
    }
}

fn is_country_acronym(s: &str) -> bool {
    let t = s.trim();
    t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic())
}

/// Parses a country input such as `"VN"` or `"-cc-vn"`, returning `(normalized_cc, force_cc_username)`.
pub fn parse_country_input(raw: &str) -> Option<(String, bool)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    for prefix in &["-cc-", "_cc_"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let cc = rest.trim();
            if is_country_acronym(cc) {
                return Some((cc.to_ascii_uppercase(), true));
            }
        }
    }
    Some((trimmed.to_ascii_uppercase(), false))
}

/// Extracts an optional inline 2-letter country acronym (`#CC`, `|CC`, `CC|`)
/// or non-country `#tag` fragment (e.g. `#oxylabs`) from the host/URL tail.
fn split_inline_country_and_tag(s: &str) -> (&str, Option<String>, Option<String>) {
    if let Some((base, frag)) = s.rsplit_once('#') {
        // Only treat `#frag` as a trailing country/tag if it is after `@` (or there is no `@`),
        // so passwords containing `#` before `@` are never truncated.
        let after_userinfo = base.rfind('@').map(|idx| &base[idx + 1..]).unwrap_or(base);
        let frag_trimmed = frag.trim();
        if !base.trim().is_empty() && !after_userinfo.is_empty() {
            if is_country_acronym(frag_trimmed) {
                return (
                    base.trim(),
                    Some(frag_trimmed.to_ascii_uppercase()),
                    None,
                );
            }
            let tag = if frag_trimmed.is_empty() {
                None
            } else {
                Some(frag_trimmed.to_string())
            };
            return (base.trim(), None, tag);
        }
    }
    if let Some((base, suffix)) = s.rsplit_once('|') {
        if is_country_acronym(suffix) && !base.trim().is_empty() {
            return (base.trim(), Some(suffix.trim().to_ascii_uppercase()), None);
        }
    }
    if let Some((prefix, rest)) = s.split_once('|') {
        if is_country_acronym(prefix) && !rest.trim().is_empty() {
            return (rest.trim(), Some(prefix.trim().to_ascii_uppercase()), None);
        }
    }
    (s, None, None)
}

type ParsedProxyComponents = (ProxyProtocol, String, u16, Option<ProxyAuth>, Option<String>);

/// Helper to parse URL or vendor string `host:port:user:pass[:CC]`
fn parse_proxy_components(s: &str) -> Result<ParsedProxyComponents, String> {
    if s.contains("://") {
        let parts: Vec<&str> = s.splitn(2, "://").collect();
        let protocol = ProxyProtocol::parse(parts[0]);
        let rest = parts[1];

        let (auth, host_port) = if let Some(idx) = rest.find('@') {
            let auth_part = &rest[..idx];
            let hp = &rest[idx + 1..];
            let creds: Vec<&str> = auth_part.splitn(2, ':').collect();
            let auth = if creds.len() == 2 {
                Some(ProxyAuth {
                    username: url_decode(creds[0]),
                    password: url_decode(creds[1]),
                })
            } else {
                None
            };
            (auth, hp)
        } else {
            (None, rest)
        };

        let clean_hp = host_port.split(['/', '?', '#']).next().unwrap_or(host_port);
        let (host, port) = parse_host_port(clean_hp)?;

        Ok((protocol, host, port, auth, None))
    } else if s.starts_with('[') {
        let (host, remainder) = if let Some(close_idx) = s.find(']') {
            (s[..=close_idx].to_string(), &s[close_idx + 1..])
        } else {
            return Err("Mismatched brackets in IPv6 address".to_string());
        };
        let rest = remainder.strip_prefix(':').unwrap_or(remainder);
        let segments: Vec<&str> = rest.split(':').collect();
        match segments.len() {
            1 => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None, None))
            }
            3 => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[1].to_string(),
                    password: segments[2].to_string(),
                });
                Ok((ProxyProtocol::Http, host, port, auth, None))
            }
            4 if is_country_acronym(segments[3]) => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[1].to_string(),
                    password: segments[2].to_string(),
                });
                Ok((
                    ProxyProtocol::Http,
                    host,
                    port,
                    auth,
                    Some(segments[3].to_ascii_uppercase()),
                ))
            }
            _ => Err(
                "Invalid IPv6 proxy format. Expected '[ipv6]:port' or '[ipv6]:port:user:pass[:CC]'"
                    .to_string(),
            ),
        }
    } else {
        // Vendor format: host:port:user:pass[:CC] or host:port
        let segments: Vec<&str> = s.split(':').collect();
        match segments.len() {
            2 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None, None))
            }
            4 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[2].to_string(),
                    password: segments[3].to_string(),
                });
                Ok((ProxyProtocol::Http, host, port, auth, None))
            }
            5 if is_country_acronym(segments[4]) => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[2].to_string(),
                    password: segments[3].to_string(),
                });
                Ok((
                    ProxyProtocol::Http,
                    host,
                    port,
                    auth,
                    Some(segments[4].to_ascii_uppercase()),
                ))
            }
            _ => Err(
                "Invalid proxy format. Expected 'protocol://host:port' or 'host:port:user:pass[:CC]'"
                    .to_string(),
            ),
        }
    }
}

fn parse_host_port(hp: &str) -> Result<(String, u16), String> {
    if hp.starts_with('[') {
        if let Some(close_idx) = hp.find(']') {
            let host = hp[..=close_idx].to_string();
            let remainder = &hp[close_idx + 1..];
            let port = if let Some(stripped) = remainder.strip_prefix(':') {
                stripped
                    .parse::<u16>()
                    .map_err(|_| "Invalid proxy port".to_string())?
            } else {
                8080
            };
            Ok((host, port))
        } else {
            Err("Mismatched brackets in IPv6 address".to_string())
        }
    } else {
        let hp_parts: Vec<&str> = hp.split(':').collect();
        if hp_parts.is_empty() || hp_parts[0].is_empty() {
            return Err("Invalid proxy host".to_string());
        }
        let host = hp_parts[0].to_string();
        let port: u16 = if hp_parts.len() > 1 {
            hp_parts[1]
                .parse()
                .map_err(|_| "Invalid proxy port".to_string())?
        } else {
            8080
        };
        Ok((host, port))
    }
}

/// Playwright / Puppeteer browser launch proxy dictionary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaywrightProxyConfig {
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Upstream proxy rotation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyRotationStrategy {
    /// Simple round-robin across enabled healthy proxies.
    #[default]
    RoundRobin,
    /// Random selection from pool.
    Random,
    /// Sticky session: same session key routes to the same proxy until failure.
    StickySession,
    /// Selects proxy with the lowest failure count.
    LowestFailures,
}

impl ProxyRotationStrategy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "round_robin" | "roundrobin" => Some(Self::RoundRobin),
            "random" => Some(Self::Random),
            "sticky_session" | "sticky" => Some(Self::StickySession),
            "lowest_failures" | "health" | "healthy" => Some(Self::LowestFailures),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::Random => "random",
            Self::StickySession => "sticky_session",
            Self::LowestFailures => "lowest_failures",
        }
    }
}

/// Query criteria when selecting an upstream proxy from the pool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxySelectionQuery {
    pub session_id: Option<String>,
    pub country: Option<String>,
    pub tag: Option<String>,
    pub preferred_id: Option<String>,
    pub strategy: Option<ProxyRotationStrategy>,
}

fn url_decode(s: &str) -> String {
    let mut decoded = Vec::with_capacity(s.len());
    let mut bytes = s.as_bytes().iter();
    while let Some(&b) = bytes.next() {
        if b == b'%' {
            if let (Some(&h1), Some(&h2)) = (bytes.next(), bytes.next()) {
                let hex_str = [h1, h2];
                if let Ok(hex) = std::str::from_utf8(&hex_str) {
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        decoded.push(byte);
                        continue;
                    }
                }
                decoded.push(b'%');
                decoded.push(h1);
                decoded.push(h2);
            } else {
                decoded.push(b'%');
            }
        } else {
            decoded.push(b);
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| s.to_string())
}

fn url_encode_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";
                out.push(HEX_CHARS[(b >> 4) as usize] as char);
                out.push(HEX_CHARS[(b & 0x0F) as usize] as char);
            }
        }
    }
    out
}

fn extract_country_from_username(username: &str) -> Option<String> {
    let u = username.to_ascii_lowercase();
    for pattern in &["-cc-", "_cc_", "-country-", "_country-", "-zone-"] {
        if let Some(pos) = u.find(pattern) {
            let after = &u[pos + pattern.len()..];
            let code: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if code.len() == 2 {
                return Some(code.to_ascii_uppercase());
            }
        }
    }
    None
}

fn replace_country_in_username(username: &str, target_country: &str) -> Option<String> {
    let u_lower = username.to_ascii_lowercase();
    let target = target_country.trim().to_ascii_lowercase();
    if target.len() != 2 || !target.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    for pattern in &["-cc-", "_cc_", "-country-", "_country-", "-zone-"] {
        if let Some(pos) = u_lower.find(pattern) {
            let prefix = &username[..pos + pattern.len()];
            let remainder = &username[pos + pattern.len()..];
            let code_len = remainder
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .count();
            if code_len == 2 {
                let suffix = &remainder[code_len..];
                return Some(format!("{prefix}{target}{suffix}"));
            }
        }
    }
    if (u_lower.starts_with("customer-") || u_lower.contains("resi")) && !u_lower.contains("-cc-") {
        return Some(format!("{username}-cc-{target}"));
    }
    None
}
