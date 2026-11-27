//! Safe Bounded HTTP Response Cache (No MITM).
//!
//! Clean Architecture: Infrastructure Adapter Layer.
//! Caches strictly safe, repeatable plain-HTTP `GET`/`HEAD` responses (e.g. static API metadata,
//! school/organization lookups) with strict opt-in configuration, bounded LRU eviction,
//! `ETag` / `If-None-Match` revalidation, and mandatory bypass rules for auth/write/sensitive flows.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

const SENSITIVE_PATH_KEYWORDS: &[&str] = &[
    "login", "signin", "signup", "auth", "oauth", "token", "verify", "submit", "checkout",
    "password", "session",
];

fn default_max_entries() -> usize {
    512
}

fn default_max_body_bytes() -> usize {
    1_048_576 // 1 MiB
}

fn default_ttl_secs() -> u64 {
    300
}

/// Opt-in configuration for the bounded HTTP response cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpCacheConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default = "default_ttl_secs")]
    pub default_ttl_secs: u64,
    #[serde(default)]
    pub domain_ttl_overrides: HashMap<String, u64>,
    #[serde(default)]
    pub auth_whitelisted_domains: Vec<String>,
}

impl Default for HttpCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: default_max_entries(),
            max_body_bytes: default_max_body_bytes(),
            default_ttl_secs: default_ttl_secs(),
            domain_ttl_overrides: HashMap::new(),
            auth_whitelisted_domains: Vec::new(),
        }
    }
}

/// Cached raw HTTP/1.1 response bytes with expiration and optional `ETag`.
#[derive(Debug, Clone)]
pub struct CachedHttpResponse {
    pub raw_response: Vec<u8>,
    pub etag: Option<String>,
    pub expires_at: Instant,
    last_access_seq: u64,
}

/// Result of querying the HTTP cache for a request key.
#[derive(Debug, Clone)]
pub enum CacheLookup {
    Hit(CachedHttpResponse),
    Revalidate {
        etag: String,
        entry: CachedHttpResponse,
    },
    Miss,
}

struct CacheState {
    config: HttpCacheConfig,
    entries: HashMap<String, CachedHttpResponse>,
    access_seq: u64,
}

/// Thread-safe bounded LRU HTTP response cache.
pub struct HttpResponseCache {
    state: RwLock<CacheState>,
}

impl HttpResponseCache {
    pub fn new(config: HttpCacheConfig) -> Self {
        let cap = config.max_entries.min(4096);
        Self {
            state: RwLock::new(CacheState {
                config,
                entries: HashMap::with_capacity(cap),
                access_seq: 0,
            }),
        }
    }

    pub fn config(&self) -> HttpCacheConfig {
        self.state
            .read()
            .map(|s| s.config.clone())
            .unwrap_or_default()
    }

    pub fn update_config(&self, config: HttpCacheConfig) {
        if let Ok(mut s) = self.state.write() {
            if !config.enabled {
                s.entries.clear();
            } else {
                evict_to_limit(&mut s.entries, config.max_entries);
            }
            s.config = config;
        }
    }

    pub fn clear(&self) {
        if let Ok(mut s) = self.state.write() {
            s.entries.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.state.read().map(|s| s.entries.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Checks whether a request is eligible for HTTP caching (mandatory safety gates).
    pub fn is_request_cacheable(&self, method: &str, url: &str, request_headers: &[u8]) -> bool {
        let state = match self.state.read() {
            Ok(s) => s,
            Err(_) => return false,
        };
        if !state.config.enabled {
            return false;
        }
        // Only GET and HEAD are safe to cache
        if !method.eq_ignore_ascii_case("GET") && !method.eq_ignore_ascii_case("HEAD") {
            return false;
        }
        // Mandatory bypass for sensitive URL paths
        if has_sensitive_path(url) {
            return false;
        }
        // Mandatory bypass if Authorization or Cookie header is present (unless host whitelisted)
        if has_auth_or_cookie_header(request_headers) {
            let host = crate::rules_engine::extract_host(url);
            let whitelisted = state
                .config
                .auth_whitelisted_domains
                .iter()
                .any(|d| host_matches_domain(&host, d));
            if !whitelisted {
                return false;
            }
        }
        // Respect client Cache-Control: no-store
        if request_has_no_store(request_headers) {
            return false;
        }
        true
    }

    /// Builds a deterministic cache key from method and normalized URL.
    pub fn cache_key(method: &str, url: &str) -> String {
        format!("{}:{}", method.to_ascii_uppercase(), url.trim())
    }

    /// Looks up a cached response, returning `Hit`, `Revalidate` (if expired with ETag), or `Miss`.
    pub fn lookup(&self, key: &str) -> CacheLookup {
        let mut state = match self.state.write() {
            Ok(s) => s,
            Err(_) => return CacheLookup::Miss,
        };
        if !state.config.enabled {
            return CacheLookup::Miss;
        }
        let now = Instant::now();
        let next_seq = state.access_seq.wrapping_add(1);
        state.access_seq = next_seq;

        let expired_without_etag = match state.entries.get_mut(key) {
            Some(entry) => {
                entry.last_access_seq = next_seq;
                if now < entry.expires_at {
                    return CacheLookup::Hit(entry.clone());
                }
                if let Some(ref etag) = entry.etag {
                    return CacheLookup::Revalidate {
                        etag: etag.clone(),
                        entry: entry.clone(),
                    };
                }
                true
            }
            None => false,
        };

        if expired_without_etag {
            state.entries.remove(key);
        }
        CacheLookup::Miss
    }

    /// Stores a raw HTTP response if it satisfies status, size, and `Cache-Control` safety rules.
    pub fn store(&self, key: &str, url: &str, raw_response: &[u8]) -> bool {
        let mut state = match self.state.write() {
            Ok(s) => s,
            Err(_) => return false,
        };
        if !state.config.enabled || state.config.max_entries == 0 {
            return false;
        }
        let Some(header_end) = crate::proxy_server::utils::find_header_end(raw_response) else {
            return false;
        };
        let body_len = raw_response.len().saturating_sub(header_end);
        if body_len > state.config.max_body_bytes {
            return false;
        }
        let Ok(header_str) = std::str::from_utf8(&raw_response[..header_end]) else {
            return false;
        };
        let Some(status_line) = header_str.lines().next() else {
            return false;
        };
        if !is_cacheable_status(status_line) {
            return false;
        }
        let (allow_cache, max_age_opt, etag) = parse_response_cache_headers(header_str);
        if !allow_cache {
            return false;
        }

        let host = crate::rules_engine::extract_host(url);
        let ttl_secs = resolve_ttl_secs(&state.config, &host, max_age_opt);
        if ttl_secs == 0 {
            return false;
        }

        let next_seq = state.access_seq.wrapping_add(1);
        state.access_seq = next_seq;
        let max_entries = state.config.max_entries;
        if state.entries.len() >= max_entries && !state.entries.contains_key(key) {
            evict_to_limit(&mut state.entries, max_entries.saturating_sub(1));
        }

        state.entries.insert(
            key.to_string(),
            CachedHttpResponse {
                raw_response: raw_response.to_vec(),
                etag,
                expires_at: Instant::now() + Duration::from_secs(ttl_secs),
                last_access_seq: next_seq,
            },
        );
        true
    }

    /// Refreshes TTL for a cached entry after an upstream `304 Not Modified` response.
    pub fn refresh_not_modified(&self, key: &str, url: &str) -> Option<CachedHttpResponse> {
        let mut state = self.state.write().ok()?;
        let host = crate::rules_engine::extract_host(url);
        let ttl_secs = resolve_ttl_secs(&state.config, &host, None);
        let next_seq = state.access_seq.wrapping_add(1);
        state.access_seq = next_seq;
        let entry = state.entries.get_mut(key)?;
        entry.expires_at = Instant::now() + Duration::from_secs(ttl_secs.max(1));
        entry.last_access_seq = next_seq;
        Some(entry.clone())
    }
}

fn evict_to_limit(entries: &mut HashMap<String, CachedHttpResponse>, max_entries: usize) {
    while entries.len() > max_entries {
        let oldest_key = entries
            .iter()
            .min_by_key(|(_, v)| v.last_access_seq)
            .map(|(k, _)| k.clone());
        match oldest_key {
            Some(k) => {
                entries.remove(&k);
            }
            None => break,
        }
    }
}

fn host_matches_domain(host: &str, domain: &str) -> bool {
    let clean = domain.trim().trim_start_matches("*.").to_ascii_lowercase();
    if clean.is_empty() {
        return false;
    }
    host.eq_ignore_ascii_case(&clean)
        || (host.ends_with(&clean)
            && host
                .strip_suffix(&clean)
                .is_some_and(|prefix| prefix.ends_with('.')))
}

fn has_sensitive_path(url: &str) -> bool {
    let without_proto = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path_and_query = without_proto
        .find('/')
        .map(|idx| &without_proto[idx..])
        .unwrap_or("");
    let path_only = path_and_query
        .split('?')
        .next()
        .unwrap_or(path_and_query)
        .to_ascii_lowercase();

    SENSITIVE_PATH_KEYWORDS
        .iter()
        .any(|kw| path_only.contains(kw))
}

fn has_auth_or_cookie_header(request_headers: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(request_headers) else {
        return true; // Fail closed on non-UTF8 headers
    };
    for line in s.lines().skip(1) {
        if let Some((name, _)) = line.split_once(':') {
            let lower = name.trim().to_ascii_lowercase();
            if lower == "authorization" || lower == "cookie" {
                return true;
            }
        }
    }
    false
}

fn request_has_no_store(request_headers: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(request_headers) else {
        return true;
    };
    for line in s.lines().skip(1) {
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("cache-control")
                && val.to_ascii_lowercase().contains("no-store")
            {
                return true;
            }
        }
    }
    false
}

fn is_cacheable_status(status_line: &str) -> bool {
    let mut parts = status_line.split_whitespace();
    let _proto = parts.next();
    matches!(parts.next(), Some("200"))
}

fn parse_response_cache_headers(header_str: &str) -> (bool, Option<u64>, Option<String>) {
    let mut allow = true;
    let mut max_age: Option<u64> = None;
    let mut etag: Option<String> = None;

    for line in header_str.lines().skip(1) {
        if let Some((name, val)) = line.split_once(':') {
            let name_lower = name.trim().to_ascii_lowercase();
            let val_trimmed = val.trim();
            match name_lower.as_str() {
                "cache-control" => {
                    let lower = val_trimmed.to_ascii_lowercase();
                    for directive in lower.split(',') {
                        let d = directive.trim();
                        if d == "no-store" || d == "private" {
                            allow = false;
                        } else if let Some(secs_str) = d.strip_prefix("max-age=") {
                            if let Ok(secs) = secs_str.trim().parse::<u64>() {
                                if secs == 0 {
                                    allow = false;
                                } else {
                                    max_age = Some(secs);
                                }
                            }
                        }
                    }
                }
                "pragma" => {
                    if val_trimmed.to_ascii_lowercase().contains("no-cache") {
                        allow = false;
                    }
                }
                "etag" => {
                    if !val_trimmed.is_empty() {
                        etag = Some(val_trimmed.to_string());
                    }
                }
                "set-cookie" => {
                    // Never cache responses that set session/auth cookies
                    allow = false;
                }
                _ => {}
            }
        }
    }

    (allow, max_age, etag)
}

fn resolve_ttl_secs(config: &HttpCacheConfig, host: &str, max_age_opt: Option<u64>) -> u64 {
    for (domain, ttl) in &config.domain_ttl_overrides {
        if host_matches_domain(host, domain) {
            return *ttl;
        }
    }
    max_age_opt
        .map(|m| m.min(config.default_ttl_secs.max(60)))
        .unwrap_or(config.default_ttl_secs)
}

/// Checks if a raw HTTP response starts with status `304 Not Modified`.
pub fn is_not_modified_response(raw_response: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(raw_response) else {
        return false;
    };
    let Some(first_line) = s.lines().next() else {
        return false;
    };
    let mut parts = first_line.split_whitespace();
    let _proto = parts.next();
    matches!(parts.next(), Some("304"))
}
