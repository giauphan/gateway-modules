//! Application Orchestrator Service.
//!
//! Clean Architecture: Application Layer (Use Cases).
//! Orchestrates the domain rules engine, metrics recording, and storage ports.

use crate::domain::{
    FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto, PatternType,
    ProxyRotationStrategy, ProxySelectionQuery, Rule, RuleAction, RuleCategory, UpstreamProxy,
};
use crate::ports::{
    FlowOptimizerPort, MetricsPort, MetricsStoragePort, ProxyPoolManagementPort,
    RuleManagementPort, RuleRepositoryPort,
};
use crate::rules_engine::RulesEngine;
use crate::script_generator::ScriptGenerator;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub struct ProxyGuardService {
    rule_repo: Arc<dyn RuleRepositoryPort>,
    metrics_repo: Arc<dyn MetricsStoragePort>,
    proxy_pool: Arc<dyn ProxyPoolManagementPort>,
    cost_per_gb_usd: RwLock<f64>,
    cached_engine: RwLock<RulesEngine>,
    cached_metrics: RwLock<FlowMetrics>,
    dirty_metrics_count: AtomicU64,
    rule_edit_lock: Mutex<()>,
    metrics_lock: Mutex<()>,
}

impl ProxyGuardService {
    pub fn new(
        rule_repo: Arc<dyn RuleRepositoryPort>,
        metrics_repo: Arc<dyn MetricsStoragePort>,
        proxy_pool: Arc<dyn ProxyPoolManagementPort>,
        cost_per_gb_usd: f64,
    ) -> Self {
        let rules = rule_repo.load_rules();
        let whitelist = rule_repo.load_whitelist();
        let engine = RulesEngine::new(rules, whitelist);
        let mut metrics = metrics_repo.load_metrics();
        metrics.cost_per_gb_usd = cost_per_gb_usd;

        Self {
            rule_repo,
            metrics_repo,
            proxy_pool,
            cost_per_gb_usd: RwLock::new(cost_per_gb_usd),
            cached_engine: RwLock::new(engine),
            cached_metrics: RwLock::new(metrics),
            dirty_metrics_count: AtomicU64::new(0),
            rule_edit_lock: Mutex::new(()),
            metrics_lock: Mutex::new(()),
        }
    }

    pub fn proxy_pool(&self) -> &Arc<dyn ProxyPoolManagementPort> {
        &self.proxy_pool
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
        Ok(())
    }

    /// Persists the latest in-memory metrics under the metrics lock, preventing
    /// out-of-order writes or stale snapshots overwriting newer state.
    fn persist_latest_metrics(&self) {
        if let Ok(_guard) = self.metrics_lock.lock() {
            if let Ok(metrics) = self.cached_metrics.read() {
                let snapshot = metrics.clone();
                drop(metrics);
                if let Err(e) = self.metrics_repo.save_metrics(&snapshot) {
                    eprintln!("[proxy-guard] Warning: Failed to persist metrics: {e}");
                }
            }
        }
    }

    fn rebuild_engine_locked(&self) {
        let rules = self.rule_repo.load_rules();
        let whitelist = self.rule_repo.load_whitelist();
        let new_engine = RulesEngine::new(rules, whitelist);
        if let Ok(mut lock) = self.cached_engine.write() {
            *lock = new_engine;
        }
    }
}

impl FlowOptimizerPort for ProxyGuardService {
    fn evaluate_url(&self, url: &str, method: &str, resource_type: Option<&str>) -> FilterDecision {
        let decision = if let Ok(engine) = self.cached_engine.read() {
            engine.evaluate(url, method, resource_type)
        } else {
            RulesEngine::with_builtin_rules().evaluate(url, method, resource_type)
        };

        // Automatically record to metrics
        self.record_decision(&decision, None);
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
            } else {
                forwarded_count = forwarded_count.saturating_add(1);
                optimized_bytes = optimized_bytes.saturating_add(req_size);
            }

            recorded_items.push((decision.clone(), req_size));
            decisions.push(decision);
        }

        // Aggregate flow decisions into metrics once per flow (off hot path)
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

impl RuleManagementPort for ProxyGuardService {
    fn get_rules(&self) -> Vec<Rule> {
        self.rule_repo.load_rules()
    }

    fn toggle_category(&self, category: RuleCategory, enabled: bool) -> Result<(), String> {
        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        let mut rules = self.rule_repo.load_rules();
        for r in &mut rules {
            if r.category == category {
                r.enabled = enabled;
            }
        }
        self.rule_repo.save_rules(&rules)?;
        self.rebuild_engine_locked();
        Ok(())
    }

    fn add_whitelist_domain(&self, domain: &str) -> Result<(), String> {
        let clean = domain.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return Ok(());
        }

        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        let mut wl = self.rule_repo.load_whitelist();
        if !wl.contains(&clean) {
            wl.push(clean);
            self.rule_repo.save_whitelist(&wl)?;
            self.rebuild_engine_locked();
        }
        Ok(())
    }

    fn add_blocked_domain(&self, domain: &str, name: Option<&str>) -> Result<(), String> {
        let clean = domain.trim().to_ascii_lowercase();
        if clean.is_empty() {
            return Ok(());
        }

        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        let rule_name = name.unwrap_or(&clean).to_string();
        let rule = Rule::new(
            format!("custom:{}", clean),
            rule_name,
            RuleCategory::Custom,
            PatternType::DomainSuffix,
            clean,
            RuleAction::Block204,
            120, // high priority for custom additions
            "User defined custom blocked domain",
        );

        let mut rules = self.rule_repo.load_rules();
        rules.retain(|r| r.id != rule.id);
        rules.push(rule);
        self.rule_repo.save_rules(&rules)?;
        self.rebuild_engine_locked();
        Ok(())
    }
}

impl MetricsPort for ProxyGuardService {
    fn record_decision(&self, decision: &FilterDecision, actual_bytes: Option<u64>) {
        let should_persist = if let Ok(mut metrics) = self.cached_metrics.write() {
            metrics.record_decision(decision, actual_bytes);
            let count = self.dirty_metrics_count.fetch_add(1, Ordering::Relaxed);
            count >= 64
        } else {
            false
        };

        if should_persist {
            self.dirty_metrics_count.store(0, Ordering::Relaxed);
            self.persist_latest_metrics();
        }
    }

    fn get_metrics(&self) -> FlowMetrics {
        self.cached_metrics
            .read()
            .map(|m| m.clone())
            .unwrap_or_else(|_| self.metrics_repo.load_metrics())
    }

    fn set_cost_per_gb(&self, rate: f64) -> Result<(), String> {
        if rate < 0.0 || !rate.is_finite() {
            return Err("cost_per_gb_usd must be a non-negative finite number".to_string());
        }
        let _guard = self.metrics_lock.lock().map_err(|e| e.to_string())?;
        let mut metrics_lock = self.cached_metrics.write().map_err(|e| e.to_string())?;
        let mut updated = metrics_lock.clone();
        updated.cost_per_gb_usd = rate;
        let gb_saved = updated.bytes_saved as f64 / (1024.0 * 1024.0 * 1024.0);
        updated.estimated_cost_saved_usd = gb_saved * rate;

        // Save to persistence first
        self.metrics_repo.save_metrics(&updated)?;

        // Commit in-memory only after save succeeds
        *metrics_lock = updated;
        if let Ok(mut lock) = self.cost_per_gb_usd.write() {
            *lock = rate;
        }
        self.dirty_metrics_count.store(0, Ordering::Relaxed);
        Ok(())
    }

    fn reset_metrics(&self) {
        if let Ok(_guard) = self.metrics_lock.lock() {
            if let Ok(mut metrics_lock) = self.cached_metrics.write() {
                let rate = self.cost_per_gb_usd.read().map(|r| *r).unwrap_or(3.5);
                let blank = FlowMetrics::new(rate);
                if self.metrics_repo.save_metrics(&blank).is_ok() {
                    *metrics_lock = blank;
                    self.dirty_metrics_count.store(0, Ordering::Relaxed);
                }
            }
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
