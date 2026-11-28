//! Rule Management & Metrics Port Implementations for `ProxyGuardService`.
//!
//! Clean Architecture: Application Layer (Submodule of `service`).

use super::ProxyGuardService;
use crate::domain::{
    DefaultRoute, FilterDecision, FlowMetrics, PatternType, RequestLogEntry, Rule, RuleAction,
    RuleCategory,
};
use crate::ports::{MetricsPort, RuleManagementPort};
use std::sync::atomic::Ordering;

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
        let trimmed = domain.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(());
        }
        let clean = trimmed.strip_prefix("*.").unwrap_or(&trimmed).to_string();

        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        let rule_name = name.unwrap_or(&clean).to_string();
        let rule = Rule::new(
            format!("custom:{clean}"),
            rule_name,
            RuleCategory::Custom,
            PatternType::DomainSuffix,
            clean,
            RuleAction::Block204,
            120,
            "User defined custom blocked domain",
        );

        let mut rules = self.rule_repo.load_rules();
        rules.retain(|r| r.id != rule.id);
        rules.push(rule);
        self.rule_repo.save_rules(&rules)?;
        self.rebuild_engine_locked();
        Ok(())
    }

    fn add_direct_bypass_domain(&self, domain: &str, name: Option<&str>) -> Result<(), String> {
        let trimmed = domain.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(());
        }
        let clean = trimmed.strip_prefix("*.").unwrap_or(&trimmed).to_string();

        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        let rule_name = name.unwrap_or(&clean).to_string();
        let rule = Rule::new(
            format!("direct:{clean}"),
            rule_name,
            RuleCategory::Custom,
            PatternType::DomainSuffix,
            clean,
            RuleAction::BypassDirect,
            125,
            "User defined direct-bypass (split-routing) domain",
        );

        let mut rules = self.rule_repo.load_rules();
        rules.retain(|r| r.id != rule.id);
        rules.push(rule);
        self.rule_repo.save_rules(&rules)?;
        self.rebuild_engine_locked();
        Ok(())
    }

    fn add_pool_route_domain(
        &self,
        domain: &str,
        pool: &str,
        name: Option<&str>,
    ) -> Result<(), String> {
        let trimmed_domain = domain.trim().to_ascii_lowercase();
        let trimmed_pool = pool.trim().to_string();
        if trimmed_domain.is_empty() || trimmed_pool.is_empty() {
            return Err("Both domain and pool must be non-empty".to_string());
        }

        let (pattern_type, clean, priority) =
            if let Some(suffix) = trimmed_domain.strip_prefix("*.") {
                (PatternType::DomainSuffix, suffix.to_string(), 130)
            } else {
                (PatternType::ExactDomain, trimmed_domain, 140)
            };

        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        let rule_name = name
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("{clean} -> {trimmed_pool}"));
        let rule_id = format!(
            "pool:{}:{}",
            if pattern_type == PatternType::ExactDomain {
                "exact"
            } else {
                "suffix"
            },
            clean
        );
        let rule = Rule::new(
            rule_id,
            rule_name,
            RuleCategory::Custom,
            pattern_type,
            clean,
            RuleAction::RoutePool(trimmed_pool),
            priority,
            "User defined domain split-routing rule to named proxy pool",
        );

        let mut rules = self.rule_repo.load_rules();
        rules.retain(|r| r.id != rule.id);
        rules.push(rule);
        self.rule_repo.save_rules(&rules)?;
        self.rebuild_engine_locked();
        Ok(())
    }

    fn set_default_route(&self, route: DefaultRoute) -> Result<(), String> {
        let _guard = self.rule_edit_lock.lock().map_err(|e| e.to_string())?;
        self.rule_repo.save_default_route(&route)?;
        if let Ok(mut lock) = self.default_route.write() {
            *lock = route;
        }
        self.rebuild_engine_locked();
        Ok(())
    }

    fn get_default_route(&self) -> DefaultRoute {
        self.default_route
            .read()
            .map(|r| r.clone())
            .unwrap_or_default()
    }
}

impl MetricsPort for ProxyGuardService {
    fn record_decision(&self, decision: &FilterDecision, actual_bytes: Option<u64>) {
        if let Ok(mut metrics) = self.cached_metrics.write() {
            metrics.record_decision(decision, actual_bytes);
            drop(metrics);
            self.mark_metrics_dirty();
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

        self.metrics_repo.save_metrics(&updated)?;

        *metrics_lock = updated;
        if let Ok(mut lock) = self.cost_per_gb_usd.write() {
            *lock = rate;
        }
        self.dirty_metrics_count.store(0, Ordering::Relaxed);
        Ok(())
    }

    fn reset_metrics(&self) {
        self.clear_request_logs();
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

    fn log_request(&self, entry: RequestLogEntry) {
        let snapshot: Option<Vec<RequestLogEntry>> = if let Ok(mut logs) = self.request_logs.write()
        {
            if logs.len() >= 100 {
                logs.pop_front();
            }
            logs.push_back(entry);
            Some(logs.iter().cloned().collect())
        } else {
            None
        };
        if let Some(items) = snapshot {
            if let Err(e) = self.metrics_repo.save_request_logs(&items) {
                eprintln!("[proxy-guard] Warning: Failed to persist request logs: {e}");
            }
        }
    }

    fn get_request_logs(&self, limit: usize) -> Vec<RequestLogEntry> {
        if let Ok(logs) = self.request_logs.read() {
            let count = if limit == 0 { 50 } else { limit.min(100) };
            logs.iter().rev().take(count).cloned().collect()
        } else {
            Vec::new()
        }
    }

    fn clear_request_logs(&self) {
        if let Ok(mut logs) = self.request_logs.write() {
            logs.clear();
        }
        if let Err(e) = self.metrics_repo.save_request_logs(&[]) {
            eprintln!("[proxy-guard] Warning: Failed to clear persisted request logs: {e}");
        }
    }

    fn record_upstream_transfer(&self, pool_key: &str, bytes_tx: u64, bytes_rx: u64) {
        if let Ok(mut metrics) = self.cached_metrics.write() {
            metrics.record_upstream_transfer(pool_key, bytes_tx, bytes_rx);
            drop(metrics);
            self.mark_metrics_dirty();
        }
    }

    fn record_cache_hit(&self, bytes_saved: u64) {
        if let Ok(mut metrics) = self.cached_metrics.write() {
            metrics.record_cache_hit(bytes_saved);
            drop(metrics);
            self.mark_metrics_dirty();
        }
    }

    fn record_cache_miss(&self) {
        if let Ok(mut metrics) = self.cached_metrics.write() {
            metrics.record_cache_miss();
            drop(metrics);
            self.mark_metrics_dirty();
        }
    }
}
