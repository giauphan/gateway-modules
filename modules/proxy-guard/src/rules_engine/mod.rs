//! Rules Matching Engine and Builtin Catalog.
//!
//! Clean Architecture: Domain Rules Engine.
//! Contains comprehensive, battle-tested filters for high-impact bandwidth waste.

pub mod catalog;

use crate::domain::{FilterDecision, ResourceType, Rule, RuleAction, RuleCategory};
use crate::mock_factory::MockFactory;
pub use catalog::builtin_rules;

pub struct RulesEngine {
    rules: Vec<Rule>,
    whitelist: Vec<String>,
}

impl RulesEngine {
    pub fn new(mut rules: Vec<Rule>, whitelist: Vec<String>) -> Self {
        // Pre-sort rules descending by priority once at engine creation
        rules.sort_by_key(|a| std::cmp::Reverse(a.priority));
        Self { rules, whitelist }
    }

    pub fn with_builtin_rules() -> Self {
        Self::new(catalog::builtin_rules(), Vec::new())
    }

    pub fn builtin_rules() -> Vec<Rule> {
        catalog::builtin_rules()
    }

    /// Evaluates a request against whitelist and active rules.
    pub fn evaluate(&self, url: &str, method: &str, resource_hint: Option<&str>) -> FilterDecision {
        let clean_url = url.trim();
        let host = extract_host(clean_url);
        let resource_type = ResourceType::infer(clean_url, resource_hint);
        let method_upper = method.to_ascii_uppercase();

        // 1. Whitelist check (overrides all blocking rules)
        for wl in &self.whitelist {
            let clean_wl = wl.trim().to_ascii_lowercase();
            if host.eq_ignore_ascii_case(&clean_wl)
                || (host.ends_with(&clean_wl)
                    && host
                        .strip_suffix(&clean_wl)
                        .is_some_and(|p| p.ends_with('.')))
            {
                return FilterDecision {
                    url: clean_url.to_string(),
                    method: method_upper,
                    resource_type,
                    action: RuleAction::ForwardUpstream,
                    matched_rule_id: None,
                    category: None,
                    estimated_bytes_saved: 0,
                    mock: None,
                    reason: format!("Host '{host}' is explicitly whitelisted"),
                };
            }
        }

        // 2. Iterate through pre-sorted rules (higher priority first) with zero cloning
        for rule in &self.rules {
            if rule.matches(&host, clean_url) {
                let mock = MockFactory::create_mock(&rule.action, clean_url);
                let saved = match rule.category {
                    RuleCategory::HeavyMedia => resource_type.typical_byte_size(),
                    RuleCategory::Font => 120_000,
                    RuleCategory::Analytics
                    | RuleCategory::Advertising
                    | RuleCategory::Telemetry => match resource_type {
                        ResourceType::Script => 85_000,
                        ResourceType::Beacon | ResourceType::Image => 2_500,
                        _ => 15_000,
                    },
                    RuleCategory::Custom => 20_000,
                };

                return FilterDecision {
                    url: clean_url.to_string(),
                    method: method_upper,
                    resource_type,
                    action: rule.action.clone(),
                    matched_rule_id: Some(rule.id.clone()),
                    category: Some(rule.category),
                    estimated_bytes_saved: saved,
                    mock,
                    reason: format!("Matched rule '{}' ({:?})", rule.name, rule.category),
                };
            }
        }

        // 3. Fallthrough: forward to upstream proxy
        FilterDecision {
            url: clean_url.to_string(),
            method: method_upper,
            resource_type,
            action: RuleAction::ForwardUpstream,
            matched_rule_id: None,
            category: None,
            estimated_bytes_saved: 0,
            mock: None,
            reason: "No blocking rule matched; forwarded to upstream proxy".to_string(),
        }
    }
}

/// Helper to parse host from full URL.
pub fn extract_host(url: &str) -> String {
    let without_proto = if let Some(idx) = url.find("://") {
        &url[idx + 3..]
    } else {
        url
    };

    let host_part = without_proto.split('/').next().unwrap_or(without_proto);
    let host_no_port = host_part.split(':').next().unwrap_or(host_part);
    host_no_port.to_ascii_lowercase()
}
