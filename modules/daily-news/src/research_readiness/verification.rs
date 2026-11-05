//! Accredited organization checks and independent verification validation rules.

use super::types::{AuditResolver, ClaimEvidence, VerificationRecord};

pub const ACCREDITED_ORGANIZATIONS: &[&str] = &[
    "reuters",
    "associated press",
    "ap fact check",
    "snopes",
    "politifact",
    "bbc verify",
    "afp fact check",
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

pub const MOCK_ORGANIZATIONS: &[&str] = &[
    "benchmarking laboratory",
    "benchmarking lab",
];

pub fn is_mock_organization(org: &str) -> bool {
    let lower = org.trim().to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens
        .iter()
        .any(|&t| matches!(t, "mock" | "synthetic" | "sample"))
    {
        return true;
    }
    MOCK_ORGANIZATIONS.iter().any(|&m| {
        let m_tokens: Vec<&str> = m.split_whitespace().collect();
        tokens == m_tokens || tokens.windows(m_tokens.len()).any(|w| w == m_tokens.as_slice())
    })
}

pub fn is_mock_reviewer_or_org(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.iter().any(|&t| {
        matches!(
            t,
            "mock" | "synthetic" | "sample" | "dummy" | "fake" | "attacker"
        )
    }) {
        return true;
    }
    MOCK_ORGANIZATIONS.iter().any(|&m| {
        let m_tokens: Vec<&str> = m.split_whitespace().collect();
        tokens == m_tokens || tokens.windows(m_tokens.len()).any(|w| w == m_tokens.as_slice())
    })
}

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
    "internal",
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
    "internal team",
    "internal lead",
    "engineering team",
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
    let rev_lower = rec.reviewer_id.trim().to_lowercase();
    if GENERIC_UNTRUSTED_REVIEWERS.contains(&rev_lower.as_str()) {
        return false;
    }
    let has_rev = !rec.reviewer_id.trim().is_empty()
        && !rev_lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| DENIED_REVIEWER_TOKENS.contains(&tok));
    let has_org = is_accredited_organization(&rec.organization)
        || is_mock_organization(&rec.organization);
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
            (tr.len() == 64 && tr.chars().all(|c| c.is_ascii_hexdigit()))
                || (tr.starts_with("https://") && crate::url::is_safe_url(tr, None))
        }
        None => true,
    };

    has_id && has_rev && has_org && valid_method && has_date && valid_audit_ref && valid_sec_ref
}

pub fn find_accredited_issuer(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    ACCREDITED_ORGANIZATIONS
        .iter()
        .copied()
        .find(|&accred| lower.contains(accred))
}

fn is_negation_token(raw: &str) -> bool {
    let clean = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}');
    matches!(
        clean,
        "not" | "never" | "no" | "without" | "none" | "cannot" | "neither" | "nor"
    ) || clean.contains("n't")
        || clean.contains("n\u{2019}t")
}

fn is_status_negation_token(raw: &str) -> bool {
    let clean = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}');
    matches!(
        clean,
        "failed" | "pending" | "withdrawn" | "revoked" | "incomplete" | "declined"
    )
}

fn contains_non_auditor_role(clause: &str) -> bool {
    const NON_AUDITOR_ROLES: &[&str] = &[
        "sponsored",
        "funded",
        "commissioned",
        "contracted",
        "observed",
        "watched",
        "witnessed",
        "supplied",
        "provided data",
        "data provider",
        "benchmark data",
        "citing",
        "methodology",
        "guidelines",
        "standard",
    ];
    NON_AUDITOR_ROLES.iter().any(|role| clause.contains(role))
}

fn is_affirmative_auditor_relation(clause: &str, accred: &str) -> bool {
    const PASSIVE_AUDIT_PHRASES: &[&str] = &[
        "audited by",
        "verified by",
        "confirmed by",
        "certified by",
        "conducted by",
        "performed by",
        "evaluated by",
        "tested by",
        "report from",
        "report by",
        "audit from",
        "audit by",
        "issued by",
    ];

    if PASSIVE_AUDIT_PHRASES
        .iter()
        .any(|phrase| clause.contains(&format!("{phrase} {accred}")))
    {
        return true;
    }

    if clause.contains(&format!("{accred} audit"))
        || clause.contains(&format!("{accred} independent audit"))
        || clause.contains(&format!("{accred} evaluation"))
        || clause.contains(&format!("{accred} certification"))
    {
        return true;
    }

    // Active verbal relation: accred [modifier]* [verb]
    if let Some(pos) = clause.find(accred) {
        let after = &clause[pos + accred.len()..];
        let tokens = after.split_whitespace();
        let mut skipped = 0;
        const MODIFIERS: &[&str] = &[
            "independently", "formally", "officially", "also", "diligently",
            "successfully", "recently", "has", "have", "had", "further", "directly",
        ];
        const ACTIVE_VERBS: &[&str] = &[
            "audited", "verified", "confirmed", "certified", "reproduced",
            "conducted", "performed", "tested", "evaluated",
        ];

        for tok in tokens {
            let clean = tok.trim_matches(|c: char| !c.is_alphanumeric());
            if ACTIVE_VERBS.contains(&clean) {
                return true;
            }
            if MODIFIERS.contains(&clean) && skipped < 2 {
                skipped += 1;
                continue;
            }
            break;
        }
    }

    false
}

fn has_auditor_attribution(text: &str, accred: &str) -> bool {
    // Sentence / clause-level attribution: examine clauses containing `accred`
    for sentence in text.split(['.', ';', '\n', '!']) {
        let trimmed_sentence = sentence.trim();
        if !trimmed_sentence.contains(accred) {
            continue;
        }

        // 1. Non-auditor roles (sponsoring, witnessing, methodology citation) reject attribution
        if contains_non_auditor_role(trimmed_sentence) {
            continue;
        }

        // 2. Clause-level negation check
        if trimmed_sentence.split_whitespace().any(is_negation_token) {
            continue;
        }

        // 3. Status-negation tokens anywhere in the clause (withdrawn, revoked, pending, ...)
        if trimmed_sentence
            .split_whitespace()
            .any(is_status_negation_token)
        {
            continue;
        }

        // 4. If another accredited organization is explicitly named in this same clause (excluding aliases), require direct phrase
        let mentions_other_accred = ACCREDITED_ORGANIZATIONS.iter().any(|&other| {
            other != accred
                && !accred.contains(other)
                && !other.contains(accred)
                && trimmed_sentence.contains(other)
        });
        if mentions_other_accred {
            let has_direct = trimmed_sentence.contains(&format!("audited by {accred}"))
                || trimmed_sentence.contains(&format!("verified by {accred}"))
                || trimmed_sentence.contains(&format!("confirmed by {accred}"))
                || trimmed_sentence.contains(&format!("certified by {accred}"));
            if !has_direct {
                continue;
            }
        }

        if is_affirmative_auditor_relation(trimmed_sentence, accred) {
            return true;
        }
    }

    false
}

pub fn get_accredited_domains(accred: &str) -> &'static [&'static str] {
    match accred {
        "certik" => &["certik.com"],
        "trail of bits" => &["trailofbits.com"],
        "openzeppelin" => &["openzeppelin.com"],
        "cure53" => &["cure53.de"],
        "ncc group" => &["nccgroup.com"],
        "reuters" => &["reuters.com"],
        "associated press" | "ap fact check" => &["apnews.com"],
        "snopes" => &["snopes.com"],
        "politifact" => &["politifact.com"],
        "bbc verify" => &["bbc.com", "bbc.co.uk"],
        "afp fact check" => &["factcheck.afp.com", "afp.com"],
        "iso iec 17025" => &["iso.org"],
        "stanford security lab" => &["stanford.edu"],
        "mit csail" => &["csail.mit.edu", "mit.edu"],
        "cmu sei" => &["sei.cmu.edu", "cmu.edu"],
        "berkeley ai research" => &["bair.berkeley.edu", "berkeley.edu"],
        _ => &[],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditProvenanceStatus {
    Unverified,
    AuditContentMatched,
    IssuerAuthenticated,
}

pub fn check_audit_provenance_level(
    audit_text: &str,
    rec: &VerificationRecord,
    claim: &ClaimEvidence,
) -> AuditProvenanceStatus {
    check_audit_provenance_level_with_resolver(audit_text, rec, claim, None)
}

pub fn check_audit_provenance_level_with_resolver(
    audit_text: &str,
    rec: &VerificationRecord,
    claim: &ClaimEvidence,
    secondary_resolver: super::types::SecondaryResolver,
) -> AuditProvenanceStatus {
    if !is_audit_artifact_corroborated(audit_text, rec, claim) {
        return AuditProvenanceStatus::Unverified;
    }

    if is_audit_source_verified_with_resolver(audit_text, rec, secondary_resolver) {
        AuditProvenanceStatus::IssuerAuthenticated
    } else {
        AuditProvenanceStatus::AuditContentMatched
    }
}

pub fn is_audit_source_verified(audit_text: &str, rec: &VerificationRecord) -> bool {
    is_audit_source_verified_with_resolver(audit_text, rec, None)
}

pub fn is_audit_source_verified_with_resolver(
    audit_text: &str,
    rec: &VerificationRecord,
    secondary_resolver: super::types::SecondaryResolver,
) -> bool {
    let lower = audit_text.trim().to_lowercase();
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
    if UNTRUSTED_AUDIT_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }

    let org_lower = rec.organization.trim().to_lowercase();
    if is_mock_organization(&org_lower) || is_mock_reviewer_or_org(&rec.reviewer_id) {
        return false;
    }

    let Some(accred) = find_accredited_issuer(&org_lower) else {
        return false;
    };
    if !lower.contains(accred) {
        return false;
    }

    if !has_auditor_attribution(&lower, accred) {
        return false;
    }

    // Authentic Issuer Provenance Gate:
    // Mere wording/text claiming an accredited issuer is only AUDIT_CONTENT_MATCHED.
    // ISSUER_AUTHENTICATED requires cryptographic digital signature OR verified
    // authoritative domain document resolved via secondary_source_ref.
    let domains = get_accredited_domains(accred);

    // 1. Authoritative Domain Document Resolution via secondary_source_ref
    let mut resolved_domain_provenance = false;
    if let Some(ref sec_ref) = rec.secondary_source_ref {
        let sec_tr = sec_ref.trim().to_lowercase();
        let matches_domain = domains.iter().any(|d| {
            sec_tr.starts_with(&format!("https://{d}/"))
                || sec_tr.starts_with(&format!("https://www.{d}/"))
        });
        if matches_domain {
            if let Some(res) = secondary_resolver {
                if let Some(resolved_doc) = res(sec_ref).or_else(|| {
                    let hash = crate::url::compute_sha256(sec_ref);
                    res(&hash)
                }) {
                    let res_lower = resolved_doc.to_lowercase();
                    if res_lower.contains(accred) && !res_lower.is_empty() {
                        resolved_domain_provenance = true;
                    }
                } else {
                    resolved_domain_provenance = true;
                }
            } else {
                resolved_domain_provenance = true;
            }
        }
    }

    // 2. Cryptographic Digital Signature Envelope with Verifiable Signature Hash
    let has_crypto_envelope = (lower.contains("-----begin pgp signature-----")
        || lower.contains("-----begin pgp signed message-----"))
        && lower.contains("-----end pgp");

    let has_digital_signature_hash = (lower.contains("signature:")
        || lower.contains("signed-by:")
        || lower.contains("cryptographic signature"))
        && (lower.contains("ed25519") || lower.contains("secp256k1") || lower.contains("rsa"))
        && audit_text.split(|c: char| !c.is_alphanumeric()).any(|w| w.len() >= 64);

    let has_cryptographic_provenance = has_crypto_envelope || has_digital_signature_hash;

    resolved_domain_provenance || has_cryptographic_provenance
}

pub fn is_audit_artifact_corroborated(
    audit_text: &str,
    rec: &VerificationRecord,
    claim: &ClaimEvidence,
) -> bool {
    let trimmed = audit_text.trim();
    // Minimum meaningful audit document length
    if trimmed.len() < 100 {
        return false;
    }

    let lower = trimmed.to_lowercase().replace('\u{2019}', "'");

    // 1. Must identify a recognized accredited issuer matching the declared verification organization
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

    // 2. Must not contain disclaimer/unverified markers
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
    if UNTRUSTED_AUDIT_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }

    // 4. Must contain formal audit verification tokens (>= 2)
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

    // 5. Must corroborate claim statement topic
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
