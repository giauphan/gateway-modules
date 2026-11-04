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

pub type SecondaryResolver<'a> = Option<&'a dyn Fn(&str) -> Option<String>>;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VerificationRecord {
    pub record_id: String,
    pub verified_at: String,
    pub reviewer_id: String,
    pub organization: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_trail_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_source_ref: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_record: Option<VerificationRecord>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_evidence_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_rights: Option<bool>,
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

pub const ACCREDITED_ORGANIZATIONS: &[&str] = &[
    "reuters",
    "associated press",
    "ap fact check",
    "snopes",
    "politifact",
    "bbc verify",
    "afp fact check",
    "benchmarking laboratory",
    "benchmarking lab",
    "national laboratory",
    "iso iec 17025",
    "stanford security lab",
    "mit csail",
    "cmu sei",
    "berkeley ai research",
    "certik",
    "ncc group",
    "trail of bits",
    "openzeppelin",
    "cure53",
];

pub const DENIED_REVIEWER_TOKENS: &[&str] = &[
    "self",
    "unreviewed",
    "none",
    "anonymous",
    "user",
    "author",
    "unverified",
    "fake",
    "dummy",
    "attacker",
];

pub const GENERIC_UNTRUSTED_REVIEWERS: &[&str] = &[
    "qa team",
    "reviewer 1",
    "reviewer",
    "expert",
    "qa",
    "test reviewer",
    "auditor",
    "peer reviewer",
];

pub fn is_accredited_organization(org: &str) -> bool {
    let lower = org.trim().to_lowercase();
    if lower.is_empty() {
        return false;
    }
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    for &denied in DENIED_REVIEWER_TOKENS {
        if tokens.contains(&denied) {
            return false;
        }
    }
    for &accred in ACCREDITED_ORGANIZATIONS {
        let accred_tokens: Vec<&str> = accred.split_whitespace().collect();
        if !accred_tokens.is_empty() {
            if tokens == accred_tokens {
                return true;
            }
            if let Some(pos) = tokens
                .windows(accred_tokens.len())
                .position(|w| w == accred_tokens.as_slice())
            {
                if pos == 0 {
                    let suffix_tokens = &tokens[accred_tokens.len()..];
                    if suffix_tokens.iter().all(|&t| {
                        matches!(
                            t,
                            "qa" | "lab"
                                | "laboratory"
                                | "team"
                                | "division"
                                | "security"
                                | "review"
                                | "audit"
                        )
                    }) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn is_accredited_reviewer(reviewer: &str) -> bool {
    let lower = reviewer.trim().to_lowercase();
    if lower.is_empty() {
        return false;
    }

    if GENERIC_UNTRUSTED_REVIEWERS.contains(&lower.as_str()) {
        return false;
    }

    for &denied in DENIED_REVIEWER_TOKENS {
        if lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| tok == denied)
        {
            return false;
        }
    }

    is_accredited_organization(&lower)
}

pub fn is_verification_record_valid(rec: &VerificationRecord) -> bool {
    let has_id = !rec.record_id.trim().is_empty();
    let rev_lower = rec.reviewer_id.to_lowercase();
    let has_rev = !rec.reviewer_id.trim().is_empty()
        && !rev_lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| DENIED_REVIEWER_TOKENS.contains(&tok));
    let has_org = is_accredited_organization(&rec.organization);
    let valid_method = matches!(
        rec.method.trim().to_lowercase().as_str(),
        "reproducible_benchmark"
            | "dual_expert_review"
            | "cryptographic_audit"
            | "independent_replication"
            | "laboratory_verification"
            | "formal_verification"
            | "peer_review"
            | "fact_check"
    );
    let has_date = !rec.verified_at.trim().is_empty();

    let valid_audit_ref = rec.audit_trail_ref.as_deref().is_some_and(|a| {
        let tr = a.trim();
        tr.len() == 64 && tr.chars().all(|c| c.is_ascii_hexdigit())
    });

    let valid_sec_ref = match rec.secondary_source_ref.as_deref() {
        Some(s) => {
            let tr = s.trim();
            tr.len() == 64 && tr.chars().all(|c| c.is_ascii_hexdigit())
        }
        None => true,
    };

    has_id && has_rev && has_org && valid_method && has_date && valid_audit_ref && valid_sec_ref
}

pub fn is_claim_independently_verified(claim: &ClaimEvidence) -> bool {
    if claim.verification_status != "INDEPENDENTLY_VERIFIED" {
        return false;
    }
    if let Some(ref rec) = claim.verification_record {
        return is_verification_record_valid(rec);
    }
    false
}

fn snapshot_contains_excerpt(snapshot_text: &str, excerpt: &str) -> bool {
    let tr_excerpt = excerpt.trim();
    if snapshot_text.contains(tr_excerpt) {
        return true;
    }
    let norm_excerpt = tr_excerpt
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    let cleaned_snapshot = crate::source_capture::extract_main_text(snapshot_text);
    if cleaned_snapshot.contains(&norm_excerpt) {
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
    norm_clean.contains(&norm_excerpt)
}

#[inline]
fn build_readiness(status: ReadinessStatus, reason: String, missing: &str) -> ResearchReadiness {
    ResearchReadiness {
        status,
        reasons: vec![reason],
        missing_fields: if missing.is_empty() {
            vec![]
        } else {
            vec![missing.to_string()]
        },
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
    claim: &ClaimEvidence,
    snapshot_text: Option<&str>,
    secondary_resolver: SecondaryResolver,
) -> bool {
    let has_locator = r
        .locator
        .as_deref()
        .is_some_and(|loc| !loc.trim().is_empty());
    let has_excerpt = r.excerpt.trim().len() >= 10;
    let ref_str = r.source_ref.trim().to_ascii_lowercase();
    let is_primary = primary_sha == ref_str;

    if !has_locator || !has_excerpt {
        return false;
    }

    if is_primary {
        match snapshot_text {
            Some(text) => snapshot_contains_excerpt(text, &r.excerpt),
            None => true,
        }
    } else {
        let is_valid_hex = ref_str.len() == 64 && ref_str.chars().all(|ch| ch.is_ascii_hexdigit());
        let verified_check = is_claim_independently_verified(claim);
        let has_secondary_link = claim
            .verification_record
            .as_ref()
            .filter(|vr| is_verification_record_valid(vr))
            .and_then(|vr| vr.secondary_source_ref.as_deref())
            .map(|s| s.trim().to_ascii_lowercase())
            .is_some_and(|s| s == ref_str);

        if !is_valid_hex || !verified_check || !has_secondary_link {
            return false;
        }

        // Secondary evidence must be grounded in an actual stored snapshot artifact!
        match secondary_resolver {
            Some(resolver) => {
                if let Some(sec_text) = resolver(&ref_str) {
                    let computed_sha = crate::url::compute_sha256(&sec_text);
                    computed_sha == ref_str && snapshot_contains_excerpt(&sec_text, &r.excerpt)
                } else {
                    false
                }
            }
            None => false,
        }
    }
}

fn are_all_claims_grounded(
    claims: &[ClaimEvidence],
    primary_sha: &str,
    snapshot_text: Option<&str>,
    secondary_resolver: SecondaryResolver,
) -> bool {
    claims.iter().all(|c| {
        let valid_status = c.verification_status == "SOURCE_SUPPORTED"
            || c.verification_status == "INDEPENDENTLY_VERIFIED";
        valid_status
            && !c.evidence_refs.is_empty()
            && c.evidence_refs.iter().all(|r| {
                is_evidence_ref_grounded(r, primary_sha, c, snapshot_text, secondary_resolver)
            })
    })
}

fn has_valid_independent_check(claims: &[ClaimEvidence]) -> bool {
    claims.iter().any(is_claim_independently_verified)
}

const APPROVED_LICENSES: &[&str] = &[
    "cc-by",
    "cc-by-sa",
    "cc0",
    "public-domain",
    "mit",
    "apache-2.0",
    "fair-use",
    "licensed",
    "approved",
    "editorial",
    "authorized",
];

pub fn is_visual_evidence_valid(v: &VisualEvidence) -> bool {
    let asset_ref = v.asset_ref.trim();
    let source_url = v.source_url.trim();

    if asset_ref.is_empty() {
        return false;
    }
    let lower_ref = asset_ref.to_lowercase();
    if matches!(
        lower_ref.as_str(),
        "none" | "null" | "placeholder" | "dummy" | "test" | "untitled"
    ) {
        return false;
    }

    if asset_ref.contains("..")
        || asset_ref.contains('\\')
        || asset_ref.starts_with('/')
        || v.source_snapshot_ref
            .as_deref()
            .unwrap_or("")
            .contains("..")
        || v.source_snapshot_ref
            .as_deref()
            .unwrap_or("")
            .contains('\\')
    {
        return false;
    }

    if lower_ref.starts_with("javascript:")
        || lower_ref.starts_with("data:")
        || lower_ref.starts_with("file:")
    {
        return false;
    }

    let valid_kind = matches!(
        v.kind.trim().to_lowercase().as_str(),
        "chart"
            | "graph"
            | "diagram"
            | "photo"
            | "image"
            | "screenshot"
            | "video"
            | "demo"
            | "figure"
            | "infographic"
    );
    if !valid_kind {
        return false;
    }

    if source_url.is_empty() || !crate::url::is_safe_url(source_url, None) {
        return false;
    }
    let lower_url = source_url.to_lowercase();
    if lower_url.starts_with("javascript:")
        || lower_url.starts_with("data:")
        || lower_url.starts_with("file:")
    {
        return false;
    }

    let valid_prov = v.provenance_status.as_deref().is_some_and(|p| {
        matches!(
            p.trim().to_lowercase().as_str(),
            "verified" | "authoritative" | "licensed" | "approved"
        )
    });

    let approved_lic = v
        .license_status
        .as_deref()
        .is_some_and(|l| APPROVED_LICENSES.contains(&l.trim().to_lowercase().as_str()));

    let valid_checksum = v.checksum_sha256.as_deref().is_some_and(|h| {
        let trimmed = h.trim();
        trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
    });

    let valid_license_evidence = v.license_evidence_ref.as_deref().is_some_and(|r| {
        let tr = r.trim();
        !tr.is_empty() && !tr.contains("..") && !tr.contains('\\')
    });

    let verified_rights = v.verified_rights.unwrap_or(false);

    valid_prov && approved_lic && valid_checksum && valid_license_evidence && verified_rights
}

fn are_visuals_verified(visuals: &[VisualEvidence]) -> bool {
    !visuals.is_empty() && visuals.iter().all(is_visual_evidence_valid)
}

pub fn evaluate_readiness(
    primary_source: Option<&PrimarySourceEvidence>,
    snapshot_text: Option<&str>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    origin: ContentOrigin,
) -> ResearchReadiness {
    evaluate_readiness_with_snapshot_store(
        primary_source,
        snapshot_text,
        claims,
        visuals,
        origin,
        None,
    )
}

pub fn evaluate_readiness_with_snapshot_text(
    primary_source: Option<&PrimarySourceEvidence>,
    snapshot_text: Option<&str>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    origin: ContentOrigin,
) -> ResearchReadiness {
    evaluate_readiness_with_snapshot_store(
        primary_source,
        snapshot_text,
        claims,
        visuals,
        origin,
        None,
    )
}

pub fn evaluate_readiness_with_snapshot_store(
    primary_source: Option<&PrimarySourceEvidence>,
    snapshot_text: Option<&str>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    _origin: ContentOrigin,
    secondary_resolver: SecondaryResolver,
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

    if !are_all_claims_grounded(claims, &primary_sha, snapshot_text, secondary_resolver) {
        return build_readiness(
            ReadinessStatus::NeedsResearch,
            "One or more claims lack validated evidence references or are not found in primary/secondary source artifacts".to_string(),
            "valid_evidence_refs",
        );
    }

    if !has_valid_independent_check(claims) {
        return build_readiness(
            ReadinessStatus::NeedsVerification,
            "Independent verification has not been run or lacks accredited verification record with audit trail".to_string(),
            "independent_fact_check",
        );
    }

    if !are_visuals_verified(visuals) {
        return build_readiness(
            ReadinessStatus::NeedsVisualEvidence,
            "No verified visual evidence assets available (assets must have valid checksum SHA-256, approved license, license evidence reference, and verified rights)".to_string(),
            "visual_evidence",
        );
    }

    if snapshot_text.is_none() {
        return build_readiness(
            ReadinessStatus::NeedsResearch,
            "Primary source snapshot bytes must be verified against snapshot storage before video readiness".to_string(),
            "verified_snapshot_text",
        );
    }

    build_readiness(
        ReadinessStatus::VideoReady,
        "All primary source, claim, and visual verification criteria satisfied".to_string(),
        "",
    )
}
