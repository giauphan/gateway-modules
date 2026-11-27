//! Inbound and Outbound Port Trait Definitions.
//!
//! Clean Architecture: Application Layer Boundary Abstractions.
//! Ensures high-level business logic is decoupled from transport, frameworks, and storage.

use crate::domain::{
    DefaultRoute, FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto,
    ProxyRotationStrategy, ProxySelectionQuery, RequestLogEntry, Rule, RuleCategory, UpstreamProxy,
};
use crate::http_cache::{CacheLookup, CachedHttpResponse, HttpCacheConfig};

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

    /// Adds a domain to the custom direct-bypass (split-routing) list (0 GB paid proxy).
    fn add_direct_bypass_domain(&self, domain: &str, name: Option<&str>) -> Result<(), String>;

    /// Adds a domain split-routing rule directing traffic to a specific named upstream proxy pool.
    fn add_pool_route_domain(
        &self,
        domain: &str,
        pool: &str,
        name: Option<&str>,
    ) -> Result<(), String>;

    /// Sets the default fallback route when no rule matches.
    fn set_default_route(&self, route: DefaultRoute) -> Result<(), String>;

    /// Returns the currently active default fallback route.
    fn get_default_route(&self) -> DefaultRoute;
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

    /// Appends an intercepted request to the live audit log buffer.
    fn log_request(&self, entry: RequestLogEntry);

    /// Returns recent intercepted request logs up to `limit`.
    fn get_request_logs(&self, limit: usize) -> Vec<RequestLogEntry>;

    /// Clears the request log buffer.
    fn clear_request_logs(&self);

    /// Records actual metered wire bytes transferred for a pool (or `"direct"`).
    fn record_upstream_transfer(&self, pool_key: &str, bytes_tx: u64, bytes_rx: u64);

    /// Records a safe HTTP response cache hit and bytes saved.
    fn record_cache_hit(&self, bytes_saved: u64);

    /// Records a safe HTTP response cache miss.
    fn record_cache_miss(&self);
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

    /// Loads persisted default fallback route (defaults to `ForwardUpstream`).
    fn load_default_route(&self) -> DefaultRoute {
        DefaultRoute::ForwardUpstream
    }

    /// Persists default fallback route.
    fn save_default_route(&self, _route: &DefaultRoute) -> Result<(), String> {
        Ok(())
    }
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

    /// Loads persisted round-robin cursor index (defaults to 0).
    fn load_rr_counter(&self) -> usize {
        0
    }

    /// Persists round-robin cursor index so rotation works across process restarts.
    fn save_rr_counter(&self, _counter: usize) -> Result<(), String> {
        Ok(())
    }
}

/// Outward Port: Bounded HTTP response cache abstraction (DIP boundary).
pub trait HttpCachePort: Send + Sync {
    fn config(&self) -> HttpCacheConfig;
    fn update_config(&self, config: HttpCacheConfig);
    fn clear(&self);
    fn is_request_cacheable(&self, method: &str, url: &str, request_headers: &[u8]) -> bool;
    fn lookup(&self, key: &str) -> CacheLookup;
    fn store(&self, key: &str, url: &str, raw_response: &[u8]) -> bool;
    fn refresh_not_modified(&self, key: &str, url: &str) -> Option<CachedHttpResponse>;
}
