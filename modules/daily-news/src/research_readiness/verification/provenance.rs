//! Provenance classification, domain matching, and cryptographic/attestation verification.

use crate::research_readiness::types::{ClaimEvidence, SecondaryResolver, VerificationRecord};
use super::issuers::{find_accredited_issuer, get_accredited_domains, is_mock_organization, is_mock_reviewer_or_org};
use super::attribution::has_auditor_attribution;
use super::corroboration::is_audit_artifact_corroborated;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditProvenanceStatus {
    Unverified,
    AuditContentMatched,
    ResolvedDocument,
    IssuerAuthenticated,
}

pub fn check_audit_provenance_level(
    audit_text: &str,
    rec: &VerificationRecord,
    claim: &ClaimEvidence,
) -> AuditProvenanceStatus {
    check_audit_provenance_level_with_resolver(audit_text, rec, claim, None)
}

fn has_resolved_document_linkage(
    rec: &VerificationRecord,
    secondary_resolver: SecondaryResolver,
) -> bool {
    let Some(res) = secondary_resolver else {
        return false;
    };
    let Some(ref sec_ref) = rec.secondary_source_ref else {
        return false;
    };
    let doc_opt = res(sec_ref).or_else(|| {
        let hash = crate::url::compute_sha256(sec_ref);
        res(&hash)
    });
    doc_opt.is_some_and(|d| !d.trim().is_empty())
}

pub fn check_audit_provenance_level_with_resolver(
    audit_text: &str,
    rec: &VerificationRecord,
    claim: &ClaimEvidence,
    secondary_resolver: SecondaryResolver,
) -> AuditProvenanceStatus {
    if !is_audit_artifact_corroborated(audit_text, rec, claim) {
        return AuditProvenanceStatus::Unverified;
    }

    if is_audit_source_verified_with_resolver(audit_text, rec, secondary_resolver) {
        AuditProvenanceStatus::IssuerAuthenticated
    } else if has_resolved_document_linkage(rec, secondary_resolver) {
        AuditProvenanceStatus::ResolvedDocument
    } else {
        AuditProvenanceStatus::AuditContentMatched
    }
}

pub fn is_audit_source_verified(audit_text: &str, rec: &VerificationRecord) -> bool {
    is_audit_source_verified_with_resolver(audit_text, rec, None)
}

pub(crate) fn has_untrusted_audit_markers(lower: &str) -> bool {
    const UNTRUSTED_AUDIT_MARKERS: &[&str] = &[
        "not an official report",
        "self-certified",
        "self-assessment",
        "self-evaluation",
        "self-audit",
        "self-reported",
        "self-generated",
        "self-declared",
        "internal evaluation",
        "internal review only",
        "internal notes only",
        "unverified draft",
        "unofficial draft",
        "author audit",
        "author-verified",
        "user-generated",
        "unaccredited report",
        "unverified document",
        "author statement",
        "simulated report",
        "mock verification",
        "draft evaluation",
    ];
    UNTRUSTED_AUDIT_MARKERS.iter().any(|m| lower.contains(m))
}

pub(crate) fn check_authoritative_domain_match(
    sec_ref: &str,
    accred: &str,
    domains: &[&str],
    rec: &VerificationRecord,
    secondary_resolver: SecondaryResolver,
) -> bool {
    let sec_tr = sec_ref.trim().to_lowercase();
    let matches_domain = domains.iter().any(|d| {
        sec_tr.starts_with(&format!("https://{d}/"))
            || sec_tr.starts_with(&format!("https://www.{d}/"))
    });
    if !matches_domain {
        return false;
    }

    let Some(res) = secondary_resolver else {
        return false;
    };
    let resolved_doc = res(sec_ref).or_else(|| {
        let hash = crate::url::compute_sha256(sec_ref);
        res(&hash)
    });
    let Some(doc) = resolved_doc else {
        return false;
    };

    let res_lower = doc.to_lowercase();
    if has_untrusted_audit_markers(&res_lower) || res_lower.len() < 30 {
        return false;
    }

    const AUTHENTIC_REPORT_MARKERS: &[&str] = &[
        "audit report",
        "audit trail",
        "audit findings",
        "security assessment",
        "formal verification",
        "benchmark evaluation",
        "benchmark report",
        "benchmarking",
        "verification report",
        "evaluation report",
        "technical review",
        "certification report",
        "security review",
    ];
    if !AUTHENTIC_REPORT_MARKERS.iter().any(|m| res_lower.contains(m)) {
        return false;
    }

    let matches_record = rec
        .audit_trail_ref
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty() && res_lower.contains(&r.trim().to_lowercase()))
        || (!rec.record_id.trim().is_empty()
            && res_lower.contains(&rec.record_id.trim().to_lowercase()))
        || (!rec.reviewer_id.trim().is_empty()
            && res_lower.contains(&rec.reviewer_id.trim().to_lowercase()))
        || (rec.secondary_source_ref.as_deref().is_some_and(|s| {
            let report_slug = s.split('/').next_back().unwrap_or("").trim().to_lowercase();
            !report_slug.is_empty() && res_lower.contains(&report_slug)
        }));

    res_lower.contains(accred) && matches_record
}

pub(crate) fn verify_cryptographic_provenance(
    audit_text: &str,
    lower: &str,
    accred: &str,
    domains: &[&str],
    rec: &VerificationRecord,
    secondary_resolver: SecondaryResolver,
) -> bool {
    let has_crypto_envelope = (lower.contains("-----begin pgp signature-----")
        || lower.contains("-----begin pgp signed message-----"))
        && lower.contains("-----end pgp");

    let has_digital_sig_header = lower.contains("signature:")
        || lower.contains("signed-by:")
        || lower.contains("cryptographic signature");

    let has_supported_algo =
        lower.contains("ed25519") || lower.contains("secp256k1") || lower.contains("rsa");

    if !has_crypto_envelope && (!has_digital_sig_header || !has_supported_algo) {
        return false;
    }

    let has_valid_sig_token = audit_text.split(|c: char| !c.is_alphanumeric()).any(|w| {
        if w.len() >= 64 && w.chars().all(|c| c.is_ascii_hexdigit()) {
            let first_ch = w.chars().next().unwrap_or(' ');
            !w.chars().all(|c| c == first_ch)
        } else {
            false
        }
    });

    let has_audit_hash_integrity = rec.audit_trail_ref.as_deref().is_some_and(|r| {
        let tr = r.trim().to_ascii_lowercase();
        tr.len() == 64 && tr.chars().all(|c| c.is_ascii_hexdigit())
    });

    if !has_valid_sig_token || !has_audit_hash_integrity {
        return false;
    }

    rec.secondary_source_ref.as_deref().is_some_and(|sec_ref| {
        check_authoritative_domain_match(sec_ref, accred, domains, rec, secondary_resolver)
    })
}

pub(crate) fn verify_trusted_fetch_attestation(
    audit_text: &str,
    lower: &str,
    accred: &str,
    domains: &[&str],
    rec: &VerificationRecord,
    secondary_resolver: SecondaryResolver,
) -> bool {
    let Some(sec_ref) = rec.secondary_source_ref.as_deref() else {
        return false;
    };
    if !check_authoritative_domain_match(sec_ref, accred, domains, rec, secondary_resolver) {
        return false;
    }

    let has_attestation = (lower.contains("-----begin gateway fetch attestation-----")
        || lower.contains("-----begin trusted attestation-----")
        || lower.contains("gateway-attestation:")
        || lower.contains("trusted-fetch-attestation:"))
        && (lower.contains("tls_verified: true")
            || lower.contains("tls-verified: true")
            || lower.contains("attested_issuer:"));

    let has_valid_sig_token = audit_text.split(|c: char| !c.is_alphanumeric()).any(|w| {
        if w.len() >= 64 && w.chars().all(|c| c.is_ascii_hexdigit()) {
            let first_ch = w.chars().next().unwrap_or(' ');
            !w.chars().all(|c| c == first_ch)
        } else {
            false
        }
    });

    let has_audit_hash_integrity = rec.audit_trail_ref.as_deref().is_some_and(|r| {
        let tr = r.trim().to_ascii_lowercase();
        tr.len() == 64 && tr.chars().all(|c| c.is_ascii_hexdigit())
    });

    has_attestation && has_valid_sig_token && has_audit_hash_integrity
}

pub fn is_audit_source_verified_with_resolver(
    audit_text: &str,
    rec: &VerificationRecord,
    secondary_resolver: SecondaryResolver,
) -> bool {
    let lower = audit_text.trim().to_lowercase();
    if has_untrusted_audit_markers(&lower) {
        return false;
    }

    let org_lower = rec.organization.trim().to_lowercase();
    if is_mock_organization(&org_lower) || is_mock_reviewer_or_org(&rec.reviewer_id) {
        return false;
    }

    let Some(accred) = find_accredited_issuer(&org_lower) else {
        return false;
    };
    if !lower.contains(accred) || !has_auditor_attribution(&lower, accred) {
        return false;
    }

    let domains = get_accredited_domains(accred);
    verify_cryptographic_provenance(
        audit_text,
        &lower,
        accred,
        &domains,
        rec,
        secondary_resolver,
    ) || verify_trusted_fetch_attestation(
        audit_text,
        &lower,
        accred,
        &domains,
        rec,
        secondary_resolver,
    )
}
