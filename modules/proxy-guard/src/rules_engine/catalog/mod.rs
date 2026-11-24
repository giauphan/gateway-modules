//! Built-in filtering rules catalog for Proxy Guard.
//!
//! Clean Architecture: Domain Layer (Static Domain Knowledge).
//! Comprehensive, battle-tested filters for high-impact bandwidth waste.

pub mod advertising;
pub mod analytics;
pub mod media_fonts;

use crate::domain::Rule;

/// Comprehensive built-in rules catalog aggregating all domain categories.
pub fn builtin_rules() -> Vec<Rule> {
    let mut rules = Vec::with_capacity(70);
    rules.extend(analytics::analytics_rules());
    rules.extend(advertising::advertising_rules());
    rules.extend(media_fonts::media_and_font_rules());
    rules
}
