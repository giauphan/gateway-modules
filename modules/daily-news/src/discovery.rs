//! Discovery bounds validation, topic matching, and fail-closed result construction.
//!
//! Enforces deterministic date range validation and fail-closed policy gating.

use crate::date::{parse_boundary_datetime, parse_iso_to_epoch};
use crate::domain::{
    canonicalize_url, classify_topic_family, compute_content_hash, compute_title_hash, is_safe_url,
    normalize_publisher, DiscoveryResult, ModuleConfig, NormalizedCandidate, RawNewsItem,
    RawProvenance, ResearchEvidenceFields,
};
use crate::research_readiness::{evaluate_readiness_with_snapshot_store, SecondaryResolver};
use serde_json::json;
use std::collections::HashSet;

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

#[allow(clippy::too_many_arguments)]
pub fn is_candidate_eligible(
    cand: &NormalizedCandidate,
    cfg: &ModuleConfig,
    from_epoch: Option<u64>,
    cutoff: u64,
    upper_bound: u64,
    seen_content_hashes: &mut HashSet<String>,
    seen_title_hashes: &mut HashSet<String>,
    topics: Option<&[String]>,
) -> bool {
    let effective_ts = cand.updated_at.as_deref().unwrap_or(&cand.published_at_utc);
    let effective_epoch = parse_iso_to_epoch(effective_ts).unwrap_or(0);

    if effective_epoch > upper_bound {
        return false;
    }

    let enforce_latest = cfg.require_latest_news || from_epoch.is_some();
    if effective_epoch < cutoff && (enforce_latest || !cand.is_evergreen) {
        return false;
    }

    if !seen_content_hashes.insert(cand.content_hash.clone()) {
        return false;
    }
    if !seen_title_hashes.insert(cand.title_hash.clone()) {
        return false;
    }

    if let Some(topic_filter) = topics {
        if !matches_topics(cand, topic_filter) {
            return false;
        }
    }

    true
}

pub fn normalize_raw_item(
    raw: &RawNewsItem,
    snapshot_text: Option<&str>,
    secondary_resolver: SecondaryResolver,
) -> Result<NormalizedCandidate, String> {
    if raw.id.trim().is_empty() {
        return Err("Raw item missing required 'id'".to_string());
    }
    if raw.url.trim().is_empty() {
        return Err("Raw item missing required 'url'".to_string());
    }
    if raw.title.trim().is_empty() {
        return Err("Raw item missing required 'title'".to_string());
    }
    if raw.published_at.trim().is_empty() {
        return Err("Raw item missing required 'published_at'".to_string());
    }
    if !is_safe_url(&raw.url, None) {
        return Err(format!("URL violates security boundary: {}", raw.url));
    }

    let canonical = canonicalize_url(&raw.url);
    let norm_pub = normalize_publisher(raw.publisher.as_deref());
    let disc_url = raw.discussion_url.clone().or_else(|| {
        raw.url
            .contains("news.ycombinator.com")
            .then(|| raw.url.clone())
    });

    let body_str = raw.body.as_deref().or(raw.description.as_deref());
    let chash = compute_content_hash(&raw.title, body_str, &canonical);
    let thash = compute_title_hash(&raw.title);
    let topic = classify_topic_family(&raw.title, body_str.unwrap_or(""));

    let readiness = evaluate_readiness_with_snapshot_store(
        raw.primary_source.as_ref(),
        snapshot_text,
        &raw.claims,
        &raw.visual_evidence,
        raw.content_origin,
        secondary_resolver,
    );

    // Stable candidate identity deterministically derived from canonical URL (prevents caller ID spoofing)
    let url_hash = crate::url::compute_sha256(&canonical);
    let candidate_id = format!("cand_{}", &url_hash[..16]);

    Ok(NormalizedCandidate {
        candidate_id,
        canonical_url: canonical.clone(),
        original_url: raw.url.clone(),
        url: canonical.clone(),
        source_url: canonical,
        source_type: raw
            .content_kind
            .clone()
            .unwrap_or_else(|| "news".to_string()),
        title: raw.title.clone(),
        publisher: raw.publisher.clone().unwrap_or_else(|| norm_pub.clone()),
        normalized_publisher: norm_pub,
        published_at_utc: raw.published_at.clone(),
        published_at: raw.published_at.clone(),
        content_hash: chash,
        title_hash: thash,
        raw_provenance: RawProvenance {
            source_id: raw.id.clone(),
            source_feed: "daily-news".to_string(),
            extra: raw.extra.clone(),
        },
        adapter_version: env!("CARGO_PKG_VERSION").to_string(),
        topic_family: topic,
        author: raw.author.clone(),
        description: raw.description.clone(),
        body: raw.body.clone(),
        summary: raw.description.clone().or_else(|| raw.body.clone()),
        tags: raw.tags.clone(),
        language: "en".to_string(),
        image_url: None,
        is_evergreen: raw.is_evergreen,
        content_kind: raw
            .content_kind
            .clone()
            .unwrap_or_else(|| "news".to_string()),
        updated_at: raw.updated_at.clone(),
        discussion_url: disc_url,
        evidence: raw.evidence.clone(),
        independent_fact_check: None,
        scoring: None,
        content_origin: raw.content_origin,
        published_at_kind: raw.published_at_kind.as_str().to_string(),
        research: ResearchEvidenceFields {
            submission_author: raw.submission_author.clone().or_else(|| raw.author.clone()),
            article_author: raw.article_author.clone(),
            hook_origin: raw.hook_origin.clone(),
            discussion: raw.discussion.clone(),
            primary_source: raw.primary_source.clone(),
            claims: raw.claims.clone(),
            visual_evidence: raw.visual_evidence.clone(),
            research_readiness: Some(readiness),
        },
    })
}
