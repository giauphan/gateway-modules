//! Domain Entities and Value Objects for Proxy Guard.
//!
//! Clean Architecture: Core Domain Layer.
//! Pure business rules with zero transport, framework, or database dependencies.

pub mod proxy;
mod proxy_parser;
mod proxy_types;
pub mod rules;

pub use proxy::{
    parse_country_input, PlaywrightProxyConfig, ProxyAuth, ProxyProtocol, ProxyRotationStrategy,
    ProxySelectionQuery, StickySessionConfig, UpstreamProxy,
};
pub use rules::{
    DefaultRoute, FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto, MockPayload,
    PatternType, PoolTrafficStats, RequestLogEntry, ResourceType, Rule, RuleAction, RuleCategory,
};
