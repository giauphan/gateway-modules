//! Domain Entities and Value Objects for Proxy Guard.
//!
//! Clean Architecture: Core Domain Layer.
//! Pure business rules with zero transport, framework, or database dependencies.

pub mod proxy;
pub mod rules;

pub use proxy::{
    PlaywrightProxyConfig, ProxyAuth, ProxyProtocol, ProxyRotationStrategy, ProxySelectionQuery,
    UpstreamProxy,
};
pub use rules::{
    FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto, MockPayload, PatternType,
    RequestLogEntry, ResourceType, Rule, RuleAction, RuleCategory,
};
