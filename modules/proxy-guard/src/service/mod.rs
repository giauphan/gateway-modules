//! Application Orchestrator Service.
//!
//! Clean Architecture: Application Layer (Use Cases).
//! Orchestrates the domain rules engine, metrics recording, HTTP cache port, and storage ports.

mod management;

use crate::domain::{
    DefaultRoute, FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto,
    ProxyRotationStrategy, ProxySelectionQuery, RequestLogEntry, RuleAction, UpstreamProxy,
};
use crate::http_cache::{HttpCacheConfig, HttpResponseCache};
use crate::ports::{
    FlowOptimizerPort, HttpCachePort, MetricsPort, MetricsStoragePort, ProxyPoolManagementPort,
    RuleRepositoryPort,
};
use crate::rules_engine::RulesEngine;
use crate::script_generator::ScriptGenerator;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const METRICS_FLUSH_BATCH_THRESHOLD: u64 = 16;
const METRICS_FLUSH_MAX_INTERVAL: Duration = Duration::from_secs(5);

pub struct ProxyGuardService {
    pub(super) rule_repo: Arc<dyn RuleRepositoryPort>,
    pub(super) metrics_repo: Arc<dyn MetricsStoragePort>,
    pub(super) proxy_pool: Arc<dyn ProxyPoolManagementPort>,
    pub(super) cost_per_gb_usd: RwLock<f64>,
    pub(super) default_route: RwLock<DefaultRoute>,
    pub(super) cached_engine: RwLock<RulesEngine>,
    pub(super) cached_metrics: RwLock<FlowMetrics>,
    pub(super) dirty_metrics_count: AtomicU64,
    last_metrics_flush: Mutex<Instant>,
    pub(super) request_logs: RwLock<VecDeque<RequestLogEntry>>,
    http_cache: Arc<dyn HttpCachePort>,
    pub(super) rule_edit_lock: Mutex<()>,
    pub(super) metrics_lock: Mutex<()>,
}

impl ProxyGuardService {
    pub fn new(
        rule_repo: Arc<dyn RuleRepositoryPort>,
        metrics_repo: Arc<dyn MetricsStoragePort>,
        proxy_pool: Arc<dyn ProxyPoolManagementPort>,
        cost_per_gb_usd: f64,
    ) -> Self {
        let default_cache = Arc::new(HttpResponseCache::new(HttpCacheConfig::default()));
        Self::with_http_cache(
            rule_repo,
            metrics_repo,
            proxy_pool,
            default_cache,
            cost_per_gb_usd,
        )
    }

    /// Dependency-injection constructor allowing a custom `HttpCachePort` implementation.
    pub fn with_http_cache(
        rule_repo: Arc<dyn RuleRepositoryPort>,
        metrics_repo: Arc<dyn MetricsStoragePort>,
        proxy_pool: Arc<dyn ProxyPoolManagementPort>,
        http_cache: Arc<dyn HttpCachePort>,
        cost_per_gb_usd: f64,
    ) -> Self {
        let rules = rule_repo.load_rules();
        let whitelist = rule_repo.load_whitelist();
        let default_route = rule_repo.load_default_route();
        let engine = RulesEngine::with_default_route(rules, whitelist, default_route.clone());
        let mut metrics = metrics_repo.load_metrics();
        metrics.cost_per_gb_usd = cost_per_gb_usd;

        Self {
            rule_repo,
            metrics_repo,
            proxy_pool,
            cost_per_gb_usd: RwLock::new(cost_per_gb_usd),
            default_route: RwLock::new(default_route),
            cached_engine: RwLock::new(engine),
            cached_metrics: RwLock::new(metrics),
            dirty_metrics_count: AtomicU64::new(0),
            last_metrics_flush: Mutex::new(Instant::now()),
            request_logs: RwLock::new(VecDeque::with_capacity(100)),
            http_cache,
            rule_edit_lock: Mutex::new(()),
            metrics_lock: Mutex::new(()),
        }
    }

    pub fn proxy_pool(&self) -> &Arc<dyn ProxyPoolManagementPort> {
        &self.proxy_pool
    }

    pub fn http_cache(&self) -> &Arc<dyn HttpCachePort> {
        &self.http_cache
    }

    pub fn configure_cache(&self, config: HttpCacheConfig) {
        self.http_cache.update_config(config);
    }

    pub fn flush_metrics(&self) -> Result<(), String> {
        let _guard = self.metrics_lock.lock().map_err(|e| e.to_string())?;
        let metrics = self
            .cached_metrics
            .read()
            .map(|m| m.clone())
            .map_err(|e| e.to_string())?;
        self.metrics_repo.save_metrics(&metrics)?;
        self.dirty_metrics_count.store(0, Ordering::Relaxed);
        if let Ok(mut last) = self.last_metrics_flush.lock() {
            *last = Instant::now();
        }
        Ok(())
    }

    /// Marks metrics dirty and persists when either the batch threshold (16 updates)
    /// or the max flush interval (5s) is reached, so short sessions and long-lived tunnels
    /// keep `.state/metrics.json` fresh without per-request disk overhead.
    pub(super) fn mark_metrics_dirty(&self) {
        let count = self.dirty_metrics_count.fetch_add(1, Ordering::Relaxed) + 1;
        let time_elapsed = self
            .last_metrics_flush
            .lock()
            .map(|t| t.elapsed() >= METRICS_FLUSH_MAX_INTERVAL)
            .unwrap_or(false);
        if count >= METRICS_FLUSH_BATCH_THRESHOLD || time_elapsed {
            self.dirty_metrics_count.store(0, Ordering::Relaxed);
            self.persist_latest_metrics();
        }
    }

    /// Persists the latest in-memory metrics under the metrics lock, preventing
    /// out-of-order writes or stale snapshots overwriting newer state.
    pub(super) fn persist_latest_metrics(&self) {
        if let Ok(_guard) = self.metrics_lock.lock() {
            if let Ok(metrics) = self.cached_metrics.read() {
                let snapshot = metrics.clone();
                drop(metrics);
                if let Err(e) = self.metrics_repo.save_metrics(&snapshot) {
                    eprintln!("[proxy-guard] Warning: Failed to persist metrics: {e}");
                } else if let Ok(mut last) = self.last_metrics_flush.lock() {
                    *last = Instant::now();
                }
            }
        }
    }

    pub(super) fn rebuild_engine_locked(&self) {
        let rules = self.rule_repo.load_rules();
        let whitelist = self.rule_repo.load_whitelist();
        let default_route = self
            .default_route
            .read()
            .map(|r| r.clone())
            .unwrap_or_default();
        let new_engine = RulesEngine::with_default_route(rules, whitelist, default_route);
        if let Ok(mut lock) = self.cached_engine.write() {
            *lock = new_engine;
        }
    }
}

fn current_utc_time_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02} UTC")
}

impl FlowOptimizerPort for ProxyGuardService {
    fn evaluate_url(&self, url: &str, method: &str, resource_type: Option<&str>) -> FilterDecision {
        let decision = if let Ok(engine) = self.cached_engine.read() {
            engine.evaluate(url, method, resource_type)
        } else {
            RulesEngine::with_builtin_rules().evaluate(url, method, resource_type)
        };

        self.record_decision(&decision, None);

        let host_candidate = url
            .split("://")
            .nth(1)
            .unwrap_or(url)
            .split('/')
            .next()
            .unwrap_or(url);
        let host = host_candidate
            .split('?')
            .next()
            .unwrap_or(host_candidate)
            .to_string();

        let action_str = if decision.is_blocked() {
            match decision.action {
                RuleAction::Block204 => "Blocked (204)".to_string(),
                RuleAction::MockScript => "Mocked (JS Stub)".to_string(),
                RuleAction::MockPixel => "Mocked (1x1 Pixel)".to_string(),
                RuleAction::BlockEmptyJson => "Blocked (Empty JSON)".to_string(),
                RuleAction::Block403 => "Blocked (403)".to_string(),
                _ => "Blocked".to_string(),
            }
        } else {
            match decision.action {
                RuleAction::BypassDirect => "Bypassed (Direct)".to_string(),
                RuleAction::RoutePool(ref pool) => format!("Proxied (Pool: {pool})"),
                _ => "Forwarded (Upstream)".to_string(),
            }
        };

        let req_id = format!(
            "req_{}",
            self.cached_metrics
                .read()
                .map(|m| m.total_requests)
                .unwrap_or(0)
        );

        let log_entry = RequestLogEntry {
            id: req_id,
            timestamp: current_utc_time_str(),
            method: method.to_ascii_uppercase(),
            url: url.to_string(),
            host,
            action: action_str,
            category: format!("{:?}", decision.category),
            reason: decision.reason.clone(),
            bytes_saved: decision.estimated_bytes_saved,
            upstream_proxy: None,
        };
        self.log_request(log_entry);

        decision
    }

    fn optimize_flow(&self, requests: &[FlowRequestDto]) -> FlowOptimizationResult {
        let mut decisions = Vec::with_capacity(requests.len());
        let mut recorded_items = Vec::with_capacity(requests.len());
        let mut original_bytes: u64 = 0;
        let mut optimized_bytes: u64 = 0;
        let mut blocked_count: usize = 0;
        let mut forwarded_count: usize = 0;

        for req in requests {
            let method = req.method.as_deref().unwrap_or("GET");
            let r_type = req.resource_type.as_deref();

            let decision = if let Ok(engine) = self.cached_engine.read() {
                engine.evaluate(&req.url, method, r_type)
            } else {
                RulesEngine::with_builtin_rules().evaluate(&req.url, method, r_type)
            };

            let req_size = req
                .size_bytes
                .unwrap_or(decision.resource_type.typical_byte_size());
            original_bytes = original_bytes.saturating_add(req_size);

            if decision.is_blocked() {
                blocked_count = blocked_count.saturating_add(1);
            } else if decision.action == RuleAction::BypassDirect {
                // Split-routed DIRECT: 0 bytes consumed on paid upstream proxy
            } else {
                forwarded_count = forwarded_count.saturating_add(1);
                optimized_bytes = optimized_bytes.saturating_add(req_size);
            }

            recorded_items.push((decision.clone(), req_size));
            decisions.push(decision);
        }

        if let Ok(mut metrics) = self.cached_metrics.write() {
            for (d, sz) in &recorded_items {
                metrics.record_decision(d, Some(*sz));
            }
        }
        self.persist_latest_metrics();
        self.dirty_metrics_count.store(0, Ordering::Relaxed);

        let orig_mb = (original_bytes as f64) / 1_048_576.0;
        let opt_mb = (optimized_bytes as f64) / 1_048_576.0;
        let reduction = if original_bytes > 0 {
            let diff = original_bytes.saturating_sub(optimized_bytes) as f64;
            ((diff / original_bytes as f64) * 100.0 * 10.0).round() / 10.0
        } else {
            0.0
        };

        let saved_bytes = original_bytes.saturating_sub(optimized_bytes);
        let saved_gb = (saved_bytes as f64) / 1_073_741_824.0;
        let rate = self.cost_per_gb_usd.read().map(|r| *r).unwrap_or(3.5);
        let cost_saved = (saved_gb * rate * 10_000.0).round() / 10_000.0;

        FlowOptimizationResult {
            total_flow_requests: requests.len(),
            blocked_requests: blocked_count,
            forwarded_requests: forwarded_count,
            original_estimated_mb: (orig_mb * 100.0).round() / 100.0,
            optimized_estimated_mb: (opt_mb * 100.0).round() / 100.0,
            reduction_percentage: reduction,
            cost_saved_usd: cost_saved,
            decisions,
        }
    }

    fn generate_client_script(&self, target: &str) -> Result<String, String> {
        match target.to_ascii_lowercase().as_str() {
            "playwright_ts" | "playwright" | "playwright-ts" => {
                Ok(ScriptGenerator::playwright_ts())
            }
            "playwright_py" | "playwright-python" => Ok(ScriptGenerator::playwright_py()),
            "puppeteer_ts" | "puppeteer" | "puppeteer-ts" => {
                Ok(ScriptGenerator::puppeteer_ts())
            }
            "mitmproxy_py" | "mitmproxy" | "mitmproxy-addon" => {
                Ok(ScriptGenerator::mitmproxy_py())
            }
            _ => Err(format!(
                "Unsupported script target '{target}'. Supported: playwright_ts, playwright_py, puppeteer_ts, mitmproxy_py"
            )),
        }
    }
}

impl Drop for ProxyGuardService {
    fn drop(&mut self) {
        let _ = self.flush_metrics();
    }
}

impl ProxyPoolManagementPort for ProxyGuardService {
    fn add_proxy(&self, proxy: UpstreamProxy) -> Result<String, String> {
        self.proxy_pool.add_proxy(proxy)
    }

    fn replace_proxies(&self, proxies: Vec<UpstreamProxy>) -> Result<(), String> {
        self.proxy_pool.replace_proxies(proxies)
    }

    fn remove_proxy(&self, id: &str) -> Result<(), String> {
        self.proxy_pool.remove_proxy(id)
    }

    fn list_proxies(&self) -> Vec<UpstreamProxy> {
        self.proxy_pool.list_proxies()
    }

    fn select_proxy(&self, query: &ProxySelectionQuery) -> Option<UpstreamProxy> {
        self.proxy_pool.select_proxy(query)
    }

    fn select_fallback(
        &self,
        failed_id: &str,
        query: &ProxySelectionQuery,
    ) -> Option<UpstreamProxy> {
        self.proxy_pool.select_fallback(failed_id, query)
    }

    fn record_proxy_result(&self, id: &str, success: bool) {
        self.proxy_pool.record_proxy_result(id, success);
    }

    fn set_strategy(&self, strategy: ProxyRotationStrategy) -> Result<(), String> {
        self.proxy_pool.set_strategy(strategy)
    }

    fn get_strategy(&self) -> ProxyRotationStrategy {
        self.proxy_pool.get_strategy()
    }
}
