//! Inbound and Outbound Port Trait Definitions.
//!
//! Clean Architecture: Application Layer Boundary Abstractions.
//! Ensures high-level business logic is decoupled from transport, frameworks, and storage.

use crate::domain::{
    FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto, ProxyRotationStrategy,
    ProxySelectionQuery, Rule, RuleCategory, UpstreamProxy,
};

/// Inbound Port: Application operations for analyzing and optimizing traffic flows.
pub trait FlowOptimizerPort: Send + Sync {
    /// Evaluates a single URL and returns the filter decision.
    fn evaluate_url(&self, url: &str, method: &str, resource_type: Option<&str>) -> FilterDecision;

    /// Evaluates a batch of requests (e.g. from HAR or browser logs) and calculates savings.
    fn optimize_flow(&self, requests: &[FlowRequestDto]) -> FlowOptimizationResult;

    /// Generates production-ready browser automation interceptor code.
    fn generate_client_script(&self, target: &str) -> Result<String, String>;
}

/// Inbound Port: Managing filtering rules and whitelist.
pub trait RuleManagementPort: Send + Sync {
    /// Returns all currently registered rules.
    fn get_rules(&self) -> Vec<Rule>;

    /// Enables or disables all rules belonging to a category.
    fn toggle_category(&self, category: RuleCategory, enabled: bool) -> Result<(), String>;

    /// Adds a domain to the whitelist.
    fn add_whitelist_domain(&self, domain: &str) -> Result<(), String>;

    /// Adds a domain to the custom blocked list.
    fn add_blocked_domain(&self, domain: &str, name: Option<&str>) -> Result<(), String>;
}

/// Inbound Port: Tracking and querying bandwidth and cost savings.
pub trait MetricsPort: Send + Sync {
    /// Records a decision into the cumulative metrics.
    fn record_decision(&self, decision: &FilterDecision, actual_bytes: Option<u64>);

    /// Returns the current cumulative metrics.
    fn get_metrics(&self) -> FlowMetrics;

    /// Updates the cost rate per GB in USD.
    fn set_cost_per_gb(&self, rate: f64) -> Result<(), String>;

    /// Resets metrics to zero.
    fn reset_metrics(&self);
}

/// Outward Port: Storage abstraction for persisting and loading rules and configuration.
pub trait RuleRepositoryPort: Send + Sync {
    /// Loads persisted rules, falling back to builtins.
    fn load_rules(&self) -> Vec<Rule>;

    /// Persists rules to storage.
    fn save_rules(&self, rules: &[Rule]) -> Result<(), String>;

    /// Loads whitelist domains.
    fn load_whitelist(&self) -> Vec<String>;

    /// Persists whitelist domains.
    fn save_whitelist(&self, whitelist: &[String]) -> Result<(), String>;
}

/// Outward Port: Storage abstraction for cumulative metrics.
pub trait MetricsStoragePort: Send + Sync {
    /// Loads metrics from storage.
    fn load_metrics(&self) -> FlowMetrics;

    /// Persists metrics to storage.
    fn save_metrics(&self, metrics: &FlowMetrics) -> Result<(), String>;
}

/// Inbound Port: Managing upstream multi-proxy pool and rotation strategies.
pub trait ProxyPoolManagementPort: Send + Sync {
    /// Adds or updates an upstream proxy in the pool.
    fn add_proxy(&self, proxy: UpstreamProxy) -> Result<String, String>;

    /// Replaces the active proxy pool with a new set of proxies.
    fn replace_proxies(&self, proxies: Vec<UpstreamProxy>) -> Result<(), String>;

    /// Removes an upstream proxy from the pool by ID.
    fn remove_proxy(&self, id: &str) -> Result<(), String>;

    /// Lists all registered upstream proxies with their health status.
    fn list_proxies(&self) -> Vec<UpstreamProxy>;

    /// Selects an upstream proxy from the pool based on criteria (Round-Robin, Sticky, Country, Tag).
    fn select_proxy(&self, query: &ProxySelectionQuery) -> Option<UpstreamProxy>;

    /// Returns a fallback proxy if the primary proxy fails.
    fn select_fallback(
        &self,
        failed_id: &str,
        query: &ProxySelectionQuery,
    ) -> Option<UpstreamProxy>;

    /// Records the success or failure outcome of forwarding through a proxy.
    fn record_proxy_result(&self, id: &str, success: bool);

    /// Updates the rotation strategy (e.g. round_robin, random, sticky_session, lowest_failures).
    fn set_strategy(&self, strategy: ProxyRotationStrategy) -> Result<(), String>;

    /// Returns the current rotation strategy.
    fn get_strategy(&self) -> ProxyRotationStrategy;
}

/// Outward Port: Storage abstraction for upstream proxy pool persistence.
pub trait ProxyPoolStoragePort: Send + Sync {
    /// Loads configured upstream proxies from storage.
    fn load_proxies(&self) -> Vec<UpstreamProxy>;

    /// Persists upstream proxies to storage.
    fn save_proxies(&self, proxies: &[UpstreamProxy]) -> Result<(), String>;
}
