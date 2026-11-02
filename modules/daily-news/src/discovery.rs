//! Discovery bounds validation, topic matching, and fail-closed result construction.
//!
//! Enforces deterministic date range validation and fail-closed policy gating.

use crate::date::parse_boundary_datetime;
use crate::domain::{DiscoveryResult, NormalizedCandidate};
use serde_json::json;

pub fn matches_topics(cand: &NormalizedCandidate, topic_filter: &[String]) -> bool {
    let cand_topic = cand.topic_family.to_lowercase();
    topic_filter.iter().any(|req| {
        let clean = req.trim().to_lowercase();
        if clean.is_empty() {
            return false;
        }
        cand_topic == clean
            || cand_topic.contains(&clean)
            || cand.tags.iter().any(|t| t.to_lowercase() == clean)
    })
}

pub fn parse_epoch_bound(
    date_str: Option<&str>,
    is_end_of_day: bool,
) -> Result<Option<u64>, (String, String)> {
    let Some(d) = date_str else {
        return Ok(None);
    };
    parse_boundary_datetime(d, is_end_of_day)
        .map(Some)
        .map_err(|err| {
            (
                "INVALID_DATE_FORMAT".to_string(),
                format!("date error: {err}"),
            )
        })
}

pub fn validate_date_bounds(
    from_str: Option<&str>,
    to_str: Option<&str>,
) -> Result<(Option<u64>, Option<u64>), (String, String)> {
    let from_epoch = parse_epoch_bound(from_str, false)
        .map_err(|(code, msg)| (code, msg.replace("date error", "from_date error")))?;
    let to_epoch = parse_epoch_bound(to_str, true)
        .map_err(|(code, msg)| (code, msg.replace("date error", "to_date error")))?;

    if let (Some(f), Some(t)) = (from_epoch, to_epoch) {
        if f > t {
            return Err((
                "INVALID_DATE_RANGE".to_string(),
                format!("from_date ({from_str:?}) must not be later than to_date ({to_str:?})"),
            ));
        }
    }
    Ok((from_epoch, to_epoch))
}

pub fn build_fail_closed_result(
    topics: Option<&[String]>,
    from_date_str: Option<&str>,
    to_date_str: Option<&str>,
) -> DiscoveryResult {
    let mut fail_reasons = Vec::new();
    if let Some(t) = topics {
        fail_reasons.push(format!("topic filter(s): {:?}", t));
    }
    if from_date_str.is_some() || to_date_str.is_some() {
        fail_reasons.push(format!(
            "date range: [{:?}, {:?}]",
            from_date_str, to_date_str
        ));
    }
    let reason = if fail_reasons.is_empty() {
        "All candidates were consumed, stale, or dropped by safety/domain policies".to_string()
    } else {
        format!(
            "No eligible candidates matched the requested {}",
            fail_reasons.join(", ")
        )
    };
    DiscoveryResult {
        tier: "none".to_string(),
        status: "NO_ELIGIBLE_TOPIC".to_string(),
        count: 0,
        candidates: vec![],
        source_file: None,
        reason: Some(reason),
        diagnostics: json!({ "tier_3_fail_closed": true }),
    }
}
