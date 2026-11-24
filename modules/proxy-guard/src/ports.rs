//! Inbound and Outbound Port Trait Definitions.
//!
//! Clean Architecture: Application Layer Boundary Abstractions.
//! Ensures high-level business logic is decoupled from transport, frameworks, and storage.

use crate::domain::{
    FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto, Rule, RuleCategory,
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
