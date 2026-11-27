//! Domain entities and value objects for upstream proxies, routing strategies, and authentication.
//!
//! Clean Architecture: Core Domain Layer.
//! Pure business rules with zero transport, framework, or database dependencies.

pub use super::proxy_parser::parse_country_input;
use super::proxy_parser::{
    extract_country_from_username, infer_proxy_tier_tag, parse_proxy_components,
    replace_country_in_username, split_inline_country_and_tag, url_encode_userinfo,
};
pub use super::proxy_types::{
    PlaywrightProxyConfig, ProxyRotationStrategy, ProxySelectionQuery, StickySessionConfig,
};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticky_session: Option<StickySessionConfig>,
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
            if !merged_tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&t))
            {
                merged_tags.push(t);
            }
        }
        if let Some(tier_tag) =
            infer_proxy_tier_tag(&host, auth.as_ref().map(|a| a.username.as_str()))
        {
            if !merged_tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(tier_tag))
            {
                merged_tags.push(tier_tag.to_string());
            }
        }

        let inferred_pool = merged_tags.first().cloned();
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
            pool_name: inferred_pool,
            sticky_session: None,
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

    /// Returns true if this proxy belongs to the named pool (matches `pool_name` or any tag).
    pub fn matches_pool(&self, pool: &str) -> bool {
        let target = pool.trim();
        if target.is_empty() {
            return true;
        }
        if let Some(ref pn) = self.pool_name {
            if pn.eq_ignore_ascii_case(target) {
                return true;
            }
        }
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(target))
    }

    /// Returns a copy of this proxy with an explicit pool name and optional sticky session config.
    pub fn with_pool_and_sticky(
        mut self,
        pool_name: Option<String>,
        sticky_session: Option<StickySessionConfig>,
    ) -> Self {
        if let Some(pool) = pool_name {
            let trimmed = pool.trim().to_string();
            if !trimmed.is_empty() {
                if !self.tags.iter().any(|t| t.eq_ignore_ascii_case(&trimmed)) {
                    self.tags.push(trimmed.clone());
                }
                self.pool_name = Some(trimmed);
            }
        }
        if sticky_session.is_some() {
            self.sticky_session = sticky_session;
        }
        self
    }

    /// Injects a deterministic, provider-safe sticky session token into the proxy username.
    /// Uses `sticky_session.username_template` when configured; otherwise falls back to
    /// `-sessid-<hex>-sesstime-10` for residential proxies.
    pub fn with_sticky_session(&self, session_id: &str) -> Self {
        self.with_sticky_session_for(session_id, None)
    }

    /// Variant of `with_sticky_session` that also accepts an optional country override for template rendering.
    pub fn with_sticky_session_for(
        &self,
        session_id: &str,
        country_override: Option<&str>,
    ) -> Self {
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            return self.clone();
        }
        let a = match self.auth.as_ref() {
            Some(auth) => auth,
            None => return self.clone(),
        };

        // Compute deterministic 12-char alphanumeric token safe for all proxy vendors
        let digest = Sha256::digest(trimmed.as_bytes());
        let mut token = String::with_capacity(12);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for &b in &digest[..6] {
            token.push(HEX[(b >> 4) as usize] as char);
            token.push(HEX[(b & 0x0f) as usize] as char);
        }

        let new_username = if let Some(ref cfg) = self.sticky_session {
            if !cfg.enabled {
                return self.clone();
            }
            if let Some(ref tmpl) = cfg.username_template {
                let country_lower = country_override
                    .or(self.country.as_deref())
                    .unwrap_or("us")
                    .trim()
                    .to_ascii_lowercase();
                let country_upper = country_lower.to_ascii_uppercase();
                let ttl_secs = cfg.ttl_secs.max(60);
                let ttl_mins = ttl_secs.div_ceil(60);
                tmpl.replace("{base_user}", &a.username)
                    .replace("{session_id}", &token)
                    .replace("{country}", &country_lower)
                    .replace("{COUNTRY}", &country_upper)
                    .replace("{ttl_mins}", &ttl_mins.to_string())
                    .replace("{ttl_secs}", &ttl_secs.to_string())
            } else {
                let user_lower = a.username.to_ascii_lowercase();
                let base_username = if let Some(idx) = user_lower.find("-sessid-") {
                    &a.username[..idx]
                } else {
                    a.username.as_str()
                };
                let ttl_mins = cfg.ttl_secs.max(60).div_ceil(60);
                format!("{base_username}-sessid-{token}-sesstime-{ttl_mins}")
            }
        } else {
            let host_lower = self.host.to_ascii_lowercase();
            let user_lower = a.username.to_ascii_lowercase();
            let is_residential = host_lower.starts_with("pr.")
                || user_lower.starts_with("customer-")
                || user_lower.contains("resi")
                || user_lower.contains("-sessid-");
            if !is_residential {
                return self.clone();
            }
            let base_username = if let Some(idx) = user_lower.find("-sessid-") {
                &a.username[..idx]
            } else {
                a.username.as_str()
            };
            format!("{base_username}-sessid-{token}-sesstime-10")
        };

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
