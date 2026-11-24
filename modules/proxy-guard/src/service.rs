//! Application Orchestrator Service.
//!
//! Clean Architecture: Application Layer (Use Cases).
//! Orchestrates the domain rules engine, metrics recording, and storage ports.

use crate::domain::{
    FilterDecision, FlowMetrics, FlowOptimizationResult, FlowRequestDto, PatternType, Rule,
    RuleAction, RuleCategory,
};
use crate::ports::{
    FlowOptimizerPort, MetricsPort, MetricsStoragePort, RuleManagementPort, RuleRepositoryPort,
};
use crate::rules_engine::RulesEngine;
use crate::script_generator::ScriptGenerator;
use std::sync::{Arc, Mutex, RwLock};

pub struct ProxyGuardService {
    rule_repo: Arc<dyn RuleRepositoryPort>,
    metrics_repo: Arc<dyn MetricsStoragePort>,
    cost_per_gb_usd: RwLock<f64>,
    cached_engine: RwLock<RulesEngine>,
    rule_edit_lock: Mutex<()>,
    metrics_lock: Mutex<()>,
}

impl ProxyGuardService {
    pub fn new(
        rule_repo: Arc<dyn RuleRepositoryPort>,
        metrics_repo: Arc<dyn MetricsStoragePort>,
        cost_per_gb_usd: f64,
    ) -> Self {
        let rules = rule_repo.load_rules();
        let whitelist = rule_repo.load_whitelist();
        let engine = RulesEngine::new(rules, whitelist);

        Self {
            rule_repo,
            metrics_repo,
            cost_per_gb_usd: RwLock::new(cost_per_gb_usd),
            cached_engine: RwLock::new(engine),
            rule_edit_lock: Mutex::new(()),
            metrics_lock: Mutex::new(()),
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

            self.record_decision(&decision, Some(req_size));
            decisions.push(decision);
        }

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
        if let Ok(_guard) = self.metrics_lock.lock() {
            let rate = self.cost_per_gb_usd.read().map(|r| *r).unwrap_or(3.5);
            let mut metrics = self.metrics_repo.load_metrics();
            metrics.cost_per_gb_usd = rate;
            metrics.record_decision(decision, actual_bytes);
            let _ = self.metrics_repo.save_metrics(&metrics);
        }
    }

    fn get_metrics(&self) -> FlowMetrics {
        self.metrics_repo.load_metrics()
    }

    fn set_cost_per_gb(&self, rate: f64) -> Result<(), String> {
        if rate < 0.0 || !rate.is_finite() {
            return Err("cost_per_gb_usd must be a non-negative finite number".to_string());
        }
        let _guard = self.metrics_lock.lock().map_err(|e| e.to_string())?;
        if let Ok(mut lock) = self.cost_per_gb_usd.write() {
            *lock = rate;
        }
        let mut metrics = self.metrics_repo.load_metrics();
        metrics.cost_per_gb_usd = rate;
        let gb_saved = metrics.bytes_saved as f64 / (1024.0 * 1024.0 * 1024.0);
        metrics.estimated_cost_saved_usd = gb_saved * rate;
        self.metrics_repo.save_metrics(&metrics)?;
        Ok(())
    }

    fn reset_metrics(&self) {
        if let Ok(_guard) = self.metrics_lock.lock() {
            let rate = self.cost_per_gb_usd.read().map(|r| *r).unwrap_or(3.5);
            let blank = FlowMetrics::new(rate);
            let _ = self.metrics_repo.save_metrics(&blank);
        }
    }
}
