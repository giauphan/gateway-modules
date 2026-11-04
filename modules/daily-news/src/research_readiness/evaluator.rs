//! Research readiness evaluation engine and multi-stage verification pipeline.

use super::types::{
    ClaimEvidence, ContentOrigin, EvidenceRef, PrimarySourceEvidence, ReadinessStatus,
    ResearchReadiness, SecondaryResolver, VisualEvidence,
};
use super::verification::{is_claim_independently_verified, is_verification_record_valid};
use super::visual::are_visuals_verified;

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
