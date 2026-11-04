//! Accredited organization checks and independent verification validation rules.

use super::types::{ClaimEvidence, VerificationRecord};

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
