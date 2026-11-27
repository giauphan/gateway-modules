//! Rules Matching Engine and Builtin Catalog.
//!
//! Clean Architecture: Domain Rules Engine.
//! Contains comprehensive, battle-tested filters for high-impact bandwidth waste.

pub mod catalog;

use crate::domain::{
    DefaultRoute, FilterDecision, PatternType, ResourceType, Rule, RuleAction, RuleCategory,
};
use crate::mock_factory::MockFactory;
pub use catalog::builtin_rules;

pub struct RulesEngine {
    rules: Vec<Rule>,
    whitelist: Vec<String>,
    default_route: DefaultRoute,
}

impl RulesEngine {
    pub fn new(rules: Vec<Rule>, whitelist: Vec<String>) -> Self {
        Self::with_default_route(rules, whitelist, DefaultRoute::ForwardUpstream)
    }

    pub fn with_default_route(
        mut rules: Vec<Rule>,
        whitelist: Vec<String>,
        default_route: DefaultRoute,
    ) -> Self {
        // Deterministic precedence:
        // 1. Higher priority first (user rules >= 120 beat builtin catalog <= 100)
        // 2. ExactDomain beats DomainSuffix/wildcard at the same priority
        // 3. Longer / more specific pattern beats shorter pattern
        rules.sort_by_key(|r| {
            (
                std::cmp::Reverse(r.priority),
                std::cmp::Reverse(r.pattern_type == PatternType::ExactDomain),
                std::cmp::Reverse(r.pattern.len()),
            )
        });
        Self {
            rules,
            whitelist,
            default_route,
        }
    }

    pub fn with_builtin_rules() -> Self {
        Self::new(catalog::builtin_rules(), Vec::new())
    }

    pub fn builtin_rules() -> Vec<Rule> {
        catalog::builtin_rules()
    }

    pub fn default_route(&self) -> &DefaultRoute {
        &self.default_route
    }

    /// Evaluates a request against explicit split-routing rules, whitelist, built-in catalog,
    /// and configurable default fallback route.
    pub fn evaluate(&self, url: &str, method: &str, resource_hint: Option<&str>) -> FilterDecision {
        let clean_url = url.trim();
        let host = extract_host(clean_url);
        let resource_type = ResourceType::infer(clean_url, resource_hint);
        let method_upper = method.to_ascii_uppercase();

        // 1. Explicit user-defined Pool split-routing rules take precedence over generic whitelist
        for rule in &self.rules {
            if matches!(rule.action, RuleAction::RoutePool(_)) && rule.matches(&host, clean_url) {
                return FilterDecision {
                    url: clean_url.to_string(),
                    method: method_upper,
                    resource_type,
                    action: rule.action.clone(),
                    matched_rule_id: Some(rule.id.clone()),
                    category: Some(rule.category),
                    estimated_bytes_saved: 0,
                    mock: None,
                    reason: format!(
                        "Matched split-routing rule '{}' ({:?})",
                        rule.name, rule.action
                    ),
                };
            }
        }

        // 2. Whitelist check (overrides all blocking rules)
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

        // 3. Iterate through pre-sorted rules (higher priority first, exact before suffix)
        for rule in &self.rules {
            if rule.matches(&host, clean_url) {
                let mock = MockFactory::create_mock(&rule.action, clean_url);
                let saved = if matches!(
                    rule.action,
                    RuleAction::RoutePool(_) | RuleAction::ForwardUpstream
                ) {
                    0
                } else {
                    match rule.category {
                        RuleCategory::HeavyMedia => resource_type.typical_byte_size(),
                        RuleCategory::Font => 120_000,
                        RuleCategory::Analytics
                        | RuleCategory::Advertising
                        | RuleCategory::Telemetry => match resource_type {
                            ResourceType::Script => 85_000,
                            ResourceType::Beacon | ResourceType::Image => 2_500,
                            _ => 15_000,
                        },
                        RuleCategory::Custom => resource_type.typical_byte_size().max(20_000),
                    }
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

        // 4. Fallthrough: apply configurable default fallback route
        match &self.default_route {
            DefaultRoute::ForwardUpstream => FilterDecision {
                url: clean_url.to_string(),
                method: method_upper,
                resource_type,
                action: RuleAction::ForwardUpstream,
                matched_rule_id: None,
                category: None,
                estimated_bytes_saved: 0,
                mock: None,
                reason: "No blocking rule matched; forwarded to upstream proxy".to_string(),
            },
            DefaultRoute::Direct => FilterDecision {
                url: clean_url.to_string(),
                method: method_upper,
                resource_type,
                action: RuleAction::BypassDirect,
                matched_rule_id: None,
                category: None,
                estimated_bytes_saved: resource_type.typical_byte_size(),
                mock: None,
                reason: "Default route: Direct (bypass paid upstream proxy)".to_string(),
            },
            DefaultRoute::Pool(pool) => FilterDecision {
                url: clean_url.to_string(),
                method: method_upper,
                resource_type,
                action: RuleAction::RoutePool(pool.clone()),
                matched_rule_id: None,
                category: None,
                estimated_bytes_saved: 0,
                mock: None,
                reason: format!("Default route: Pool '{pool}'"),
            },
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
