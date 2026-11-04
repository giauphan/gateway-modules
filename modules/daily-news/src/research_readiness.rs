//! Research readiness, claim verification, and source provenance contracts.
//!
//! Enforces Clean Architecture domain rules: separates verified primary source
//! facts from community discussions, and guards video production readiness.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

pub fn evaluate_readiness(
    primary_source: Option<&PrimarySourceEvidence>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    origin: ContentOrigin,
) -> ResearchReadiness {
    evaluate_readiness_with_snapshot_text(primary_source, None, claims, visuals, origin)
}

pub fn evaluate_readiness_with_snapshot_text(
    primary_source: Option<&PrimarySourceEvidence>,
    snapshot_text: Option<&str>,
    claims: &[ClaimEvidence],
    visuals: &[VisualEvidence],
    _origin: ContentOrigin,
) -> ResearchReadiness {
    let mut reasons = Vec::new();
    let mut missing = Vec::new();

    let has_source = matches!(
        primary_source,
        Some(s) if s.fetch_status == "SUCCESS"
            && !s.url.trim().is_empty()
            && s.snapshot_ref
                .as_deref()
                .is_some_and(|r| !r.trim().is_empty())
            && s.snapshot_sha256.as_deref().is_some_and(|h| {
                let trimmed = h.trim();
                trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
            })
    );

    if !has_source {
        reasons.push("Primary article has not been captured or verified".to_string());
        missing.push("primary_source".to_string());
        return ResearchReadiness {
            status: ReadinessStatus::NeedsSource,
            reasons,
            missing_fields: missing,
            contract_version: "2.0".to_string(),
        };
    }

    let primary_sha = primary_source
        .and_then(|s| s.snapshot_sha256.as_deref())
        .map(|h| h.trim().to_ascii_lowercase());

    // If snapshot text is supplied, verify actual SHA-256 against declared primary hash
    if let (Some(text), Some(ref expected_sha)) = (snapshot_text, &primary_sha) {
        let computed_sha = Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if &computed_sha != expected_sha {
            reasons.push(format!(
                "Snapshot SHA-256 mismatch against stored primary source (expected {expected_sha}, computed {computed_sha})"
            ));
            missing.push("valid_snapshot_sha256".to_string());
            return ResearchReadiness {
                status: ReadinessStatus::NeedsSource,
                reasons,
                missing_fields: missing,
                contract_version: "2.0".to_string(),
            };
        }
    }

    if claims.is_empty() {
        reasons.push("No grounded claims supported by captured primary source".to_string());
        missing.push("claims".to_string());
        return ResearchReadiness {
            status: ReadinessStatus::NeedsResearch,
            reasons,
            missing_fields: missing,
            contract_version: "2.0".to_string(),
        };
    }

    // Verify evidence references and grounding
    let mut all_claims_grounded = true;
    for c in claims {
        let valid_status = c.verification_status == "SOURCE_SUPPORTED"
            || c.verification_status == "INDEPENDENTLY_VERIFIED";
        if !valid_status || c.evidence_refs.is_empty() {
            all_claims_grounded = false;
            break;
        }

        for r in &c.evidence_refs {
            let has_locator = r
                .locator
                .as_deref()
                .is_some_and(|loc| !loc.trim().is_empty());
            let has_excerpt = r.excerpt.trim().len() >= 10;
            let ref_str = r.source_ref.trim().to_ascii_lowercase();
            let matches_source = primary_sha.as_deref() == Some(&ref_str)
                || (ref_str.len() == 64 && ref_str.chars().all(|ch| ch.is_ascii_hexdigit()));

            let excerpt_in_snapshot = match snapshot_text {
                Some(text) if primary_sha.as_deref() == Some(&ref_str) => {
                    text.contains(r.excerpt.trim())
                }
                _ => true,
            };

            if !has_locator || !has_excerpt || !matches_source || !excerpt_in_snapshot {
                all_claims_grounded = false;
                break;
            }
        }
    }

    if !all_claims_grounded {
        reasons.push("One or more claims lack validated evidence references or are not found in primary source".to_string());
        missing.push("valid_evidence_refs".to_string());
        return ResearchReadiness {
            status: ReadinessStatus::NeedsResearch,
            reasons,
            missing_fields: missing,
            contract_version: "2.0".to_string(),
        };
    }

    // Independent fact-check verification:
    // Raw inputs cannot self-report INDEPENDENTLY_VERIFIED without an accredited reviewer or an independent secondary source!
    let has_valid_independent_check = claims.iter().any(|c| {
        if c.verification_status != "INDEPENDENTLY_VERIFIED" {
            return false;
        }

        let has_independent_reviewer = c.reviewer.as_deref().is_some_and(|r| {
            let tr = r.trim().to_lowercase();
            !tr.is_empty() && tr != "self" && tr != "unreviewed" && tr != "none"
        });

        let has_secondary_source_ref = c.evidence_refs.iter().any(|r| {
            let ref_str = r.source_ref.trim().to_ascii_lowercase();
            match primary_sha.as_deref() {
                Some(p_sha) => ref_str != p_sha && ref_str.len() == 64,
                None => false,
            }
        });

        has_independent_reviewer || has_secondary_source_ref
    });

    if !has_valid_independent_check {
        reasons.push("Independent verification has not been run or lacks accredited reviewer/secondary source".to_string());
        missing.push("independent_fact_check".to_string());
        return ResearchReadiness {
            status: ReadinessStatus::NeedsVerification,
            reasons,
            missing_fields: missing,
            contract_version: "2.0".to_string(),
        };
    }

    // Strict visual evidence validation:
    // Must be non-empty, safe HTTP/HTTPS URL scheme (no javascript:), verified provenance, and approved license.
    let visuals_verified = !visuals.is_empty()
        && visuals.iter().all(|v| {
            let has_asset = !v.asset_ref.trim().is_empty() || !v.source_url.trim().is_empty();
            let safe_url = if !v.source_url.trim().is_empty() {
                let trimmed = v.source_url.trim().to_lowercase();
                trimmed.starts_with("https://") || trimmed.starts_with("http://")
            } else {
                true
            };

            let valid_provenance = matches!(
                v.provenance_status
                    .as_deref()
                    .map(|s| s.trim().to_lowercase())
                    .as_deref(),
                Some("verified") | Some("authoritative") | Some("licensed") | Some("approved")
            );

            let approved_license =
                match v.license_status.as_deref().map(|s| s.trim().to_lowercase()) {
                    Some(lic) => matches!(
                        lic.as_str(),
                        "cc-by"
                            | "cc-by-sa"
                            | "cc0"
                            | "public-domain"
                            | "mit"
                            | "apache-2.0"
                            | "fair-use"
                            | "licensed"
                            | "approved"
                            | "editorial"
                            | "authorized"
                    ),
                    None => false,
                };

            has_asset && safe_url && valid_provenance && approved_license
        });

    if !visuals_verified {
        reasons.push("No verified visual evidence assets available (assets must have approved license and valid scheme)".to_string());
        missing.push("visual_evidence".to_string());
        return ResearchReadiness {
            status: ReadinessStatus::NeedsVisualEvidence,
            reasons,
            missing_fields: missing,
            contract_version: "2.0".to_string(),
        };
    }

    ResearchReadiness {
        status: ReadinessStatus::VideoReady,
        reasons: vec![
            "All primary source, claim, and visual verification criteria satisfied".to_string(),
        ],
        missing_fields: Vec::new(),
        contract_version: "2.0".to_string(),
    }
}
