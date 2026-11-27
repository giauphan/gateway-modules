//! Value Objects & Configuration Types for Upstream Proxies.
//!
//! Clean Architecture: Core Domain Layer (Submodule of `domain::proxy`).

use serde::{Deserialize, Serialize};

fn default_sticky_ttl_secs() -> u64 {
    600
}

/// Sticky session configuration for rotating residential proxies.
/// Supports dynamic username templates for DataImpulse, Oxylabs, Evomi, IPRoyal, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StickySessionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sticky_ttl_secs")]
    pub ttl_secs: u64,
    /// Optional template with placeholders: `{base_user}`, `{session_id}`, `{country}`, `{COUNTRY}`, `{ttl_mins}`, `{ttl_secs}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_template: Option<String>,
}

impl Default for StickySessionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_secs: default_sticky_ttl_secs(),
            username_template: None,
        }
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
    #[serde(default)]
    pub pool: Option<String>,
    pub preferred_id: Option<String>,
    pub strategy: Option<ProxyRotationStrategy>,
}
