//! Independent claim verification and audit artifact corroboration rules.

use crate::research_readiness::types::{AuditResolver, ClaimEvidence, VerificationRecord};
use super::issuers::{is_verification_record_valid, ACCREDITED_ORGANIZATIONS, MOCK_ORGANIZATIONS};
use super::attribution::has_auditor_attribution;
use super::provenance::has_untrusted_audit_markers;

pub fn is_audit_artifact_corroborated(
    audit_text: &str,
    rec: &VerificationRecord,
    claim: &ClaimEvidence,
) -> bool {
    let trimmed = audit_text.trim();
    if trimmed.len() < 100 {
        return false;
    }

    let lower = trimmed.to_lowercase().replace('\u{2019}', "'");

    let org_lower = rec.organization.trim().to_lowercase();
    if org_lower.is_empty() {
        return false;
    }
    let declared_accred = ACCREDITED_ORGANIZATIONS
        .iter()
        .chain(MOCK_ORGANIZATIONS.iter())
        .find(|&&a| org_lower.contains(a) && lower.contains(a));
    let Some(&accred_name) = declared_accred else {
        return false;
    };

    if !has_auditor_attribution(&lower, accred_name) {
        return false;
    }

    if has_untrusted_audit_markers(&lower) {
        return false;
    }

    const AUDIT_VERDICT_TOKENS: &[&str] = &[
        "verified",
        "audit",
        "confirmed",
        "finding",
        "findings",
        "reproduced",
        "pass",
        "benchmark",
        "accredited",
        "certik",
        "trail of bits",
        "openzeppelin",
        "laboratory",
    ];

    let verdict_count = AUDIT_VERDICT_TOKENS
        .iter()
        .filter(|tok| lower.contains(*tok))
        .count();
    if verdict_count < 2 {
        return false;
    }

    let claim_tokens: Vec<&str> = claim
        .statement
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 4)
        .collect();
    !claim_tokens.is_empty()
        && (claim_tokens
            .iter()
            .any(|tok| lower.contains(&tok.to_lowercase()))
            || lower.contains(&claim.claim_id.to_lowercase()))
}

pub fn is_claim_independently_verified_with_audit_resolver(
    claim: &ClaimEvidence,
    audit_resolver: AuditResolver,
) -> bool {
    if claim.verification_status != "INDEPENDENTLY_VERIFIED" {
        return false;
    }
    let rec = match claim.verification_record.as_ref() {
        Some(r) => r,
        None => return false,
    };

    if !is_verification_record_valid(rec) {
        return false;
    }

    if let Some(resolver) = audit_resolver {
        let audit_ref = match rec.audit_trail_ref.as_deref() {
            Some(a) => a.trim().to_ascii_lowercase(),
            None => return false,
        };

        match resolver(&audit_ref) {
            Some(audit_text) => {
                let computed = crate::url::compute_sha256(&audit_text);
                if computed != audit_ref {
                    return false;
                }
                if !is_audit_artifact_corroborated(&audit_text, rec, claim) {
                    return false;
                }
            }
            None => return false,
        }
    }

    true
}

pub fn is_claim_independently_verified(claim: &ClaimEvidence) -> bool {
    is_claim_independently_verified_with_audit_resolver(claim, None)
}
