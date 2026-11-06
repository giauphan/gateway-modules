//! Auditor attribution, semantic affirmative relations, and negation analysis.

use super::issuers::ACCREDITED_ORGANIZATIONS;

pub(crate) fn is_negation_token(raw: &str) -> bool {
    let clean = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}');
    matches!(
        clean,
        "not" | "never" | "no" | "without" | "none" | "cannot" | "neither" | "nor"
    ) || clean.contains("n't")
        || clean.contains("n\u{2019}t")
}

pub(crate) fn is_status_negation_token(raw: &str) -> bool {
    let clean = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}');
    matches!(
        clean,
        "failed" | "pending" | "withdrawn" | "revoked" | "incomplete" | "declined"
    )
}

pub(crate) fn contains_non_auditor_role(clause: &str) -> bool {
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

pub(crate) fn is_affirmative_auditor_relation(clause: &str, accred: &str) -> bool {
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

pub fn has_auditor_attribution(text: &str, accred: &str) -> bool {
    for sentence in text.split(['.', ';', '\n', '!']) {
        let trimmed_sentence = sentence.trim();
        if !trimmed_sentence.contains(accred) {
            continue;
        }

        if contains_non_auditor_role(trimmed_sentence) {
            continue;
        }

        if trimmed_sentence.split_whitespace().any(is_negation_token) {
            continue;
        }

        if trimmed_sentence
            .split_whitespace()
            .any(is_status_negation_token)
        {
            continue;
        }

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
