//! Accredited organization checks and verification record validation.

use crate::research_readiness::types::VerificationRecord;

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
