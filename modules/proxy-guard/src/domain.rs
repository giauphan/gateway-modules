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
