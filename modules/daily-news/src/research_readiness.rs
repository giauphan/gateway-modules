//! Research readiness, claim verification, and source provenance contracts.
//!
//! Enforces Clean Architecture domain rules: separates verified primary source
//! facts from community discussions, and guards video production readiness.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentOrigin {
    Article,
    HnSelfPost,
    Discussion,
    HeadlineOnly,
    #[default]
    #[serde(other)]
    Unknown,
}

impl ContentOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Article => "article",
            Self::HnSelfPost => "hn_self_post",
            Self::Discussion => "discussion",
            Self::HeadlineOnly => "headline_only",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PublishedAtKind {
    ArticlePublication,
    HnSubmission,
    FeedObserved,
    #[default]
    #[serde(other)]
    Unknown,
}

impl PublishedAtKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ArticlePublication => "article_publication",
            Self::HnSubmission => "hn_submission",
            Self::FeedObserved => "feed_observed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DiscussionCommentItem {
    pub id: String,
    pub author: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionPayload {
    pub url: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub comments: Vec<DiscussionCommentItem>,
}

impl Default for DiscussionPayload {
    fn default() -> Self {
        Self {
            url: String::new(),
            role: "community_opinion".to_string(),
            summary: None,
            comments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PrimarySourceEvidence {
    pub url: String,
    pub fetch_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EvidenceRef {
    pub source_ref: String,
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimEvidence {
    pub claim_id: String,
    pub statement: String,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
    pub verification_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VisualEvidence {
    pub asset_ref: String,
    pub kind: String,
    pub source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessStatus {
    #[default]
    Discovered,
    NeedsSource,
    NeedsResearch,
    NeedsVerification,
    NeedsVisualEvidence,
    VideoReady,
    Rejected,
}

impl ReadinessStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovered => "DISCOVERED",
            Self::NeedsSource => "NEEDS_SOURCE",
            Self::NeedsResearch => "NEEDS_RESEARCH",
            Self::NeedsVerification => "NEEDS_VERIFICATION",
            Self::NeedsVisualEvidence => "NEEDS_VISUAL_EVIDENCE",
            Self::VideoReady => "VIDEO_READY",
            Self::Rejected => "REJECTED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchReadiness {
    pub status: ReadinessStatus,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    pub contract_version: String,
}

impl Default for ResearchReadiness {
    fn default() -> Self {
        Self {
            status: ReadinessStatus::Discovered,
            reasons: Vec::new(),
            missing_fields: Vec::new(),
            contract_version: "2.0".to_string(),
        }
    }
}

pub const ACCREDITED_REVIEWER_KEYWORDS: &[&str] = &[
    "fact_checker", "fact-checker", "editorial", "editor", "lead", "analyst", "expert",
    "lab", "laboratory", "qa", "reviewer", "auditor", "peer", "researcher", "curator",
];

pub const DENIED_REVIEWER_TOKENS: &[&str] = &[
    "self", "unreviewed", "none", "anonymous", "user", "author", "unverified", "fake",
];

pub fn is_accredited_reviewer(reviewer: &str) -> bool {
    let lower = reviewer.trim().to_lowercase();
    if lower.is_empty() {
        return false;
    }

    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|s| !s.is_empty())
        .collect();

    if tokens.is_empty() {
        return false;
    }

    for token in &tokens {
        for st in token.split(['_', '-']) {
            if DENIED_REVIEWER_TOKENS.contains(&st) {
                return false;
            }
        }
    }

    tokens.iter().any(|&token| {
        ACCREDITED_REVIEWER_KEYWORDS.contains(&token)
            || token.split(['_', '-']).any(|st| ACCREDITED_REVIEWER_KEYWORDS.contains(&st))
    })
}

fn snapshot_contains_excerpt(snapshot_text: &str, excerpt: &str) -> bool {
    let tr_excerpt = excerpt.trim();
    if snapshot_text.contains(tr_excerpt) {
        return true;
    }
    let mut cleaned = String::with_capacity(snapshot_text.len());
    let mut in_tag = false;
    for c in snapshot_text.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
            cleaned.push(' ');
        } else if !in_tag {
            cleaned.push(c);
        }
    }
    let norm_clean = cleaned.split_whitespace().collect::<Vec<&str>>().join(" ");
    let norm_excerpt = tr_excerpt.split_whitespace().collect::<Vec<&str>>().join(" ");
    norm_clean.contains(&norm_excerpt)
}

pub fn evaluate_readiness(
    primary_source: Option<&PrimarySourceEvidence>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    origin: ContentOrigin,
) -> ResearchReadiness {
    evaluate_readiness_with_snapshot_text(primary_source, None, claims, visuals, origin)
}

#[inline]
fn build_readiness(status: ReadinessStatus, reason: String, missing: &str) -> ResearchReadiness {
    ResearchReadiness {
        status,
        reasons: vec![reason],
        missing_fields: if missing.is_empty() { vec![] } else { vec![missing.to_string()] },
        contract_version: "2.0".to_string(),
    }
}

fn check_primary_source(
    primary_source: Option<&PrimarySourceEvidence>,
    snapshot_text: Option<&str>,
) -> Result<String, ResearchReadiness> {
    let has_source = matches!(
        primary_source,
        Some(s) if s.fetch_status == "SUCCESS"
            && !s.url.trim().is_empty()
            && s.snapshot_ref.as_deref().is_some_and(|r| !r.trim().is_empty())
            && s.snapshot_sha256.as_deref().is_some_and(|h| {
                let trimmed = h.trim();
                trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
            })
    );

    if !has_source {
        return Err(build_readiness(
            ReadinessStatus::NeedsSource,
            "Primary article has not been captured or verified".to_string(),
            "primary_source",
        ));
    }

    let primary_sha = primary_source
        .and_then(|s| s.snapshot_sha256.as_deref())
        .map(|h| h.trim().to_ascii_lowercase())
        .unwrap_or_default();

    if let Some(text) = snapshot_text {
        let computed_sha = crate::url::compute_sha256(text);
        if computed_sha != primary_sha {
            return Err(build_readiness(
                ReadinessStatus::NeedsSource,
                format!("Snapshot SHA-256 mismatch against stored primary source (expected {primary_sha}, computed {computed_sha})"),
                "valid_snapshot_sha256",
            ));
        }
    }

    Ok(primary_sha)
}

fn is_evidence_ref_grounded(
    r: &EvidenceRef,
    primary_sha: &str,
    reviewer: Option<&str>,
    snapshot_text: Option<&str>,
) -> bool {
    let has_locator = r.locator.as_deref().is_some_and(|loc| !loc.trim().is_empty());
    let has_excerpt = r.excerpt.trim().len() >= 10;
    let ref_str = r.source_ref.trim().to_ascii_lowercase();
    let is_primary = primary_sha == ref_str;
    let is_verified_secondary = !is_primary
        && ref_str.len() == 64
        && ref_str.chars().all(|ch| ch.is_ascii_hexdigit())
        && reviewer.is_some_and(is_accredited_reviewer);

    let matches_source = is_primary || is_verified_secondary;
    let excerpt_in_snapshot = match snapshot_text {
        Some(text) if is_primary => snapshot_contains_excerpt(text, &r.excerpt),
        _ => true,
    };

    has_locator && has_excerpt && matches_source && excerpt_in_snapshot
}

fn are_all_claims_grounded(
    claims: &[ClaimEvidence],
    primary_sha: &str,
    snapshot_text: Option<&str>,
) -> bool {
    claims.iter().all(|c| {
        let valid_status = c.verification_status == "SOURCE_SUPPORTED"
            || c.verification_status == "INDEPENDENTLY_VERIFIED";
        valid_status
            && !c.evidence_refs.is_empty()
            && c.evidence_refs.iter().all(|r| {
                is_evidence_ref_grounded(r, primary_sha, c.reviewer.as_deref(), snapshot_text)
            })
    })
}

fn has_valid_independent_check(claims: &[ClaimEvidence]) -> bool {
    claims.iter().any(|c| {
        c.verification_status == "INDEPENDENTLY_VERIFIED"
            && c.reviewer.as_deref().is_some_and(is_accredited_reviewer)
    })
}

const APPROVED_LICENSES: &[&str] = &[
    "cc-by", "cc-by-sa", "cc0", "public-domain", "mit", "apache-2.0",
    "fair-use", "licensed", "approved", "editorial", "authorized",
];

fn is_visual_evidence_valid(v: &VisualEvidence) -> bool {
    let has_asset = !v.asset_ref.trim().is_empty() || !v.source_url.trim().is_empty();
    let safe_url = if !v.source_url.trim().is_empty() {
        let trimmed = v.source_url.trim().to_lowercase();
        trimmed.starts_with("https://") || trimmed.starts_with("http://")
    } else {
        true
    };

    let valid_prov = v.provenance_status.as_deref().is_some_and(|p| {
        matches!(p.trim().to_lowercase().as_str(), "verified" | "authoritative" | "licensed" | "approved")
    });

    let approved_lic = v.license_status.as_deref().is_some_and(|l| {
        APPROVED_LICENSES.contains(&l.trim().to_lowercase().as_str())
    });

    has_asset && safe_url && valid_prov && approved_lic
}

fn are_visuals_verified(visuals: &[VisualEvidence]) -> bool {
    !visuals.is_empty() && visuals.iter().all(is_visual_evidence_valid)
}

pub fn evaluate_readiness_with_snapshot_text(
    primary_source: Option<&PrimarySourceEvidence>,
    snapshot_text: Option<&str>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    _origin: ContentOrigin,
) -> ResearchReadiness {
    let primary_sha = match check_primary_source(primary_source, snapshot_text) {
        Ok(sha) => sha,
        Err(readiness) => return readiness,
    };

    if claims.is_empty() {
        return build_readiness(
            ReadinessStatus::NeedsResearch,
            "No grounded claims supported by captured primary source".to_string(),
            "claims",
        );
    }

    if !are_all_claims_grounded(claims, &primary_sha, snapshot_text) {
        return build_readiness(
            ReadinessStatus::NeedsResearch,
            "One or more claims lack validated evidence references or are not found in primary source".to_string(),
            "valid_evidence_refs",
        );
    }

    if !has_valid_independent_check(claims) {
        return build_readiness(
            ReadinessStatus::NeedsVerification,
            "Independent verification has not been run or lacks accredited reviewer/secondary source".to_string(),
            "independent_fact_check",
        );
    }

    if !are_visuals_verified(visuals) {
        return build_readiness(
            ReadinessStatus::NeedsVisualEvidence,
            "No verified visual evidence assets available (assets must have approved license and valid scheme)".to_string(),
            "visual_evidence",
        );
    }

    build_readiness(
        ReadinessStatus::VideoReady,
        "All primary source, claim, and visual verification criteria satisfied".to_string(),
        "",
    )
}
