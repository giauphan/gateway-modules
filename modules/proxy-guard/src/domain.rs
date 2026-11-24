//! Domain Entities and Value Objects for Proxy Guard.
//!
//! Clean Architecture: Core Domain Layer.
//! Pure business rules with zero transport, framework, or database dependencies.

use serde::{Deserialize, Serialize};

/// High-level category of filtering rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleCategory {
    Analytics,
    Advertising,
    Telemetry,
    HeavyMedia,
    Font,
    Custom,
}

/// Action to execute when a request matches a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    /// Return HTTP 204 No Content immediately. Zero bytes sent to upstream proxy.
    Block204,
    /// Return harmless stub JavaScript so client apps/scripts don't crash.
    MockScript,
    /// Return 1x1 transparent GIF (43 bytes).
    MockPixel,
    /// Return empty JSON `{}` with HTTP 200.
    BlockEmptyJson,
    /// Return HTTP 403 Forbidden.
    Block403,
    /// Bypass the paid upstream proxy (fetch direct via local network if safe).
    BypassDirect,
    /// Forward to paid upstream residential/datacenter proxy.
    ForwardUpstream,
}

/// Pattern matching strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    /// Matches domain and subdomains (e.g. `google-analytics.com` matches `ssl.google-analytics.com`).
    DomainSuffix,
    /// Matches exact domain only.
    ExactDomain,
    /// Matches substring within full URL.
    UrlContains,
    /// Matches file extension (e.g. `.mp4`, `.woff2`).
    Extension,
}

/// Resource type of the HTTP request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Script,
    Image,
    MediaVideo,
    Font,
    Stylesheet,
    XhrFetch,
    Document,
    Beacon,
    Other,
}

impl ResourceType {
    /// Infer resource type from URL path extension or hint.
    pub fn infer(url: &str, hint: Option<&str>) -> Self {
        if let Some(h) = hint {
            match h.to_ascii_lowercase().as_str() {
                "script" | "js" => return Self::Script,
                "image" | "img" => return Self::Image,
                "media" | "video" | "audio" => return Self::MediaVideo,
                "font" => return Self::Font,
                "stylesheet" | "css" => return Self::Stylesheet,
                "xhr" | "fetch" => return Self::XhrFetch,
                "document" | "html" => return Self::Document,
                "beacon" | "ping" => return Self::Beacon,
                _ => {}
            }
        }

        let clean_path = url.split('?').next().unwrap_or(url);
        let lower = clean_path.to_ascii_lowercase();

        if lower.ends_with(".js") || lower.ends_with(".mjs") {
            Self::Script
        } else if lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".webp")
            || lower.ends_with(".gif")
            || lower.ends_with(".svg")
            || lower.ends_with(".ico")
        {
            Self::Image
        } else if lower.ends_with(".mp4")
            || lower.ends_with(".webm")
            || lower.ends_with(".ogg")
            || lower.ends_with(".mp3")
            || lower.ends_with(".wav")
            || lower.ends_with(".mov")
        {
            Self::MediaVideo
        } else if lower.ends_with(".woff2")
            || lower.ends_with(".woff")
            || lower.ends_with(".ttf")
            || lower.ends_with(".eot")
            || lower.ends_with(".otf")
        {
            Self::Font
        } else if lower.ends_with(".css") {
            Self::Stylesheet
        } else {
            Self::Other
        }
    }

    /// Estimated average size in bytes for savings estimation.
    pub fn typical_byte_size(&self) -> u64 {
        match self {
            Self::MediaVideo => 8_000_000, // 8MB
            Self::Image => 250_000,        // 250KB
            Self::Font => 120_000,         // 120KB
            Self::Script => 85_000,        // 85KB
            Self::Stylesheet => 45_000,    // 45KB
            Self::Beacon => 500,           // 500B
            Self::XhrFetch => 2_500,       // 2.5KB
            Self::Document => 30_000,      // 30KB
            Self::Other => 10_000,         // 10KB
        }
    }
}

/// Synthetic mock response payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MockPayload {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

/// A filter rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: RuleCategory,
    pub pattern_type: PatternType,
    pub pattern: String,
    pub action: RuleAction,
    pub enabled: bool,
    pub priority: u32,
    pub description: String,
}

impl Rule {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        category: RuleCategory,
        pattern_type: PatternType,
        pattern: impl Into<String>,
        action: RuleAction,
        priority: u32,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            category,
            pattern_type,
            pattern: pattern.into().to_ascii_lowercase(),
            action,
            enabled: true,
            priority,
            description: description.into(),
        }
    }

    /// Tests if this rule matches a given host and URL path.
    pub fn matches(&self, host: &str, full_url: &str) -> bool {
        if !self.enabled {
            return false;
        }

        let clean_host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();

        match self.pattern_type {
            PatternType::ExactDomain => clean_host == self.pattern,
            PatternType::DomainSuffix => {
                clean_host == self.pattern || clean_host.ends_with(&format!(".{}", self.pattern))
            }
            PatternType::UrlContains => full_url.to_ascii_lowercase().contains(&self.pattern),
            PatternType::Extension => {
                let path = full_url.split('?').next().unwrap_or(full_url);
                path.to_ascii_lowercase().ends_with(&self.pattern)
            }
        }
    }
}

/// Outcome of evaluating a single request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterDecision {
    pub url: String,
    pub method: String,
    pub resource_type: ResourceType,
    pub action: RuleAction,
    pub matched_rule_id: Option<String>,
    pub category: Option<RuleCategory>,
    pub estimated_bytes_saved: u64,
    pub mock: Option<MockPayload>,
    pub reason: String,
}

impl FilterDecision {
    pub fn is_blocked(&self) -> bool {
        !matches!(
            self.action,
            RuleAction::ForwardUpstream | RuleAction::BypassDirect
        )
    }
}

/// Request DTO for batch flow analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRequestDto {
    pub url: String,
    pub method: Option<String>,
    pub resource_type: Option<String>,
    pub size_bytes: Option<u64>,
}

/// Cumulative flow optimization metrics and cost savings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowMetrics {
    pub total_requests: u64,
    pub blocked_requests: u64,
    pub forwarded_requests: u64,
    pub direct_bypassed_requests: u64,
    pub bytes_saved: u64,
    pub upstream_bytes_used: u64,
    pub cost_per_gb_usd: f64,
    pub estimated_cost_saved_usd: f64,
}

impl FlowMetrics {
    pub fn new(cost_per_gb_usd: f64) -> Self {
        Self {
            cost_per_gb_usd,
            ..Default::default()
        }
    }

    pub fn record_decision(&mut self, decision: &FilterDecision, actual_size: Option<u64>) {
        self.total_requests = self.total_requests.saturating_add(1);

        if decision.is_blocked() {
            self.blocked_requests = self.blocked_requests.saturating_add(1);
            let bytes = actual_size.unwrap_or(decision.estimated_bytes_saved);
            self.bytes_saved = self.bytes_saved.saturating_add(bytes);
        } else if decision.action == RuleAction::BypassDirect {
            self.direct_bypassed_requests = self.direct_bypassed_requests.saturating_add(1);
        } else {
            self.forwarded_requests = self.forwarded_requests.saturating_add(1);
            let bytes = actual_size.unwrap_or(decision.resource_type.typical_byte_size());
            self.upstream_bytes_used = self.upstream_bytes_used.saturating_add(bytes);
        }

        let saved_gb = (self.bytes_saved as f64) / 1_073_741_824.0;
        self.estimated_cost_saved_usd =
            (saved_gb * self.cost_per_gb_usd * 10_000.0).round() / 10_000.0;
    }
}

/// Summary report after optimizing a flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowOptimizationResult {
    pub total_flow_requests: usize,
    pub blocked_requests: usize,
    pub forwarded_requests: usize,
    pub original_estimated_mb: f64,
    pub optimized_estimated_mb: f64,
    pub reduction_percentage: f64,
    pub cost_saved_usd: f64,
    pub decisions: Vec<FilterDecision>,
}

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
    /// - `http://user:pass@host:port`
    /// - `socks5://host:port`
    /// - `host:port:user:pass` (standard proxy vendor format)
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

        let (protocol, host, port, auth) = parse_proxy_components(trimmed)?;

        let id = format!("{}:{}", host, port);
        let proxy_name = name.unwrap_or(&id).to_string();

        let canonical_url = match &auth {
            Some(a) => format!(
                "{}://{}:{}@{}:{}",
                protocol.scheme(),
                a.username,
                a.password,
                host,
                port
            ),
            None => format!("{}://{}:{}", protocol.scheme(), host, port),
        };

        Ok(Self {
            id,
            name: proxy_name,
            url: canonical_url,
            protocol,
            host,
            port,
            auth,
            country: country.map(|c| c.trim().to_ascii_uppercase()),
            weight: 1,
            enabled: true,
            failure_count: 0,
            success_count: 0,
            tags: tags.unwrap_or_default(),
        })
    }

    /// Formats this proxy into a standard format for client tools (Playwright, Puppeteer).
    pub fn to_playwright_config(&self) -> PlaywrightProxyConfig {
        PlaywrightProxyConfig {
            server: format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port),
            username: self.auth.as_ref().map(|a| a.username.clone()),
            password: self.auth.as_ref().map(|a| a.password.clone()),
        }
    }
}

/// Helper to parse URL or vendor string `host:port:user:pass`
fn parse_proxy_components(
    s: &str,
) -> Result<(ProxyProtocol, String, u16, Option<ProxyAuth>), String> {
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
                    username: creds[0].to_string(),
                    password: creds[1].to_string(),
                })
            } else {
                None
            };
            (auth, hp)
        } else {
            (None, rest)
        };

        let clean_hp = host_port.split('/').next().unwrap_or(host_port);
        let hp_parts: Vec<&str> = clean_hp.split(':').collect();
        if hp_parts.is_empty() {
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

        Ok((protocol, host, port, auth))
    } else {
        // Vendor format: host:port:user:pass or host:port
        let segments: Vec<&str> = s.split(':').collect();
        match segments.len() {
            2 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None))
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
                Ok((ProxyProtocol::Http, host, port, auth))
            }
            _ => Err(
                "Invalid proxy format. Expected 'protocol://host:port' or 'host:port:user:pass'"
                    .to_string(),
            ),
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

/// Query criteria when selecting an upstream proxy from the pool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxySelectionQuery {
    pub session_id: Option<String>,
    pub country: Option<String>,
    pub tag: Option<String>,
    pub preferred_id: Option<String>,
}
