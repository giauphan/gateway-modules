//! Pure domain entities and value objects for research readiness and claim verification.

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
pub type AuditResolver<'a> = Option<&'a dyn Fn(&str) -> Option<String>>;
pub type AssetResolver<'a> = Option<&'a dyn Fn(&str) -> Option<Vec<u8>>>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClaimGroundingStatus {
    #[default]
    Unverified,
    ExcerptNotFound,
    ExcerptFound,
    ClaimEntailed,
    IndependentlyVerified,
    Contradicted,
}

impl ClaimGroundingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unverified => "UNVERIFIED",
            Self::ExcerptNotFound => "EXCERPT_NOT_FOUND",
            Self::ExcerptFound => "EXCERPT_FOUND",
            Self::ClaimEntailed => "CLAIM_ENTAILED",
            Self::IndependentlyVerified => "INDEPENDENTLY_VERIFIED",
            Self::Contradicted => "CONTRADICTED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VerifiedAssetRecord {
    pub asset_id: String,
    pub checksum_sha256: String,
    pub bytes_verified: bool,
    pub license_verified: bool,
    pub verified_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_path: Option<String>,
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
