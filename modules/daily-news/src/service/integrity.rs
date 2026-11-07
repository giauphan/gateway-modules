//! Integrity and readiness verification use cases for DailyNewsService.

use super::DailyNewsService;
use crate::domain::{NormalizedCandidate, ReadinessStatus, ResearchReadiness};

pub type AuditResolver<'a> = &'a (dyn Fn(&str) -> Option<String> + 'a);

impl DailyNewsService {
    pub fn reverify_cached_readiness_if_needed(&self, cand: &mut NormalizedCandidate) -> bool {
        let is_video_ready = cand
            .research
            .research_readiness
            .as_ref()
            .is_some_and(|r| r.status == ReadinessStatus::VideoReady);

        if !is_video_ready {
            return false;
        }

        if let Some(reason) = self.check_primary_snapshot_integrity(cand) {
            cand.research.research_readiness = Some(ResearchReadiness {
                status: ReadinessStatus::NeedsRevalidation,
                reasons: vec![reason],
                missing_fields: vec!["snapshot_storage".to_string()],
                contract_version: "2.0".to_string(),
            });
            return true;
        }

        if let Some(reason) = self.check_visual_evidence_integrity(cand) {
            cand.research.research_readiness = Some(ResearchReadiness {
                status: ReadinessStatus::NeedsRevalidation,
                reasons: vec![reason],
                missing_fields: vec!["visual_storage".to_string()],
                contract_version: "2.0".to_string(),
            });
            return true;
        }

        if let Some(reason) = self.check_claims_audit_integrity(cand) {
            cand.research.research_readiness = Some(ResearchReadiness {
                status: ReadinessStatus::NeedsRevalidation,
                reasons: vec![reason],
                missing_fields: vec!["audit_storage".to_string()],
                contract_version: "2.0".to_string(),
            });
            return true;
        }

        false
    }

    fn check_primary_snapshot_integrity(&self, cand: &NormalizedCandidate) -> Option<String> {
        let ps = cand.research.primary_source.as_ref()?;
        let sha = ps.snapshot_sha256.as_ref()?;
        let text = ps
            .snapshot_ref
            .as_deref()
            .and_then(|r| self.read_snapshot_text(r))
            .or_else(|| self.read_snapshot_text(sha));
        match text {
            Some(t) => {
                let computed = crate::url::compute_sha256(&t);
                if computed != *sha {
                    Some(format!(
                        "Primary source snapshot checksum mismatch: expected {sha}, found {computed}"
                    ))
                } else {
                    None
                }
            }
            None => Some(format!("Primary source snapshot '{sha}' missing from storage")),
        }
    }

    fn check_visual_evidence_integrity(&self, cand: &NormalizedCandidate) -> Option<String> {
        for v in &cand.research.visual_evidence {
            if let Some(ref sha) = v.checksum_sha256 {
                let bytes = self.read_snapshot_bytes(sha);
                match bytes {
                    Some(b) => {
                        let computed = crate::url::compute_sha256_bytes(&b);
                        if computed != *sha {
                            return Some(format!("Visual asset '{sha}' checksum mismatch on disk"));
                        }
                    }
                    None => return Some(format!("Visual asset '{sha}' missing from storage")),
                }
            }
        }
        None
    }

    fn check_claims_audit_integrity(&self, cand: &NormalizedCandidate) -> Option<String> {
        for c in &cand.research.claims {
            if let Some(ref vr) = c.verification_record {
                if let Some(ref sha) = vr.audit_trail_ref {
                    let text = self.read_snapshot_text(sha);
                    match text {
                        Some(t) => {
                            let computed = crate::url::compute_sha256(&t);
                            if computed != *sha {
                                return Some(format!(
                                    "Audit trail artifact '{sha}' checksum mismatch on disk"
                                ));
                            }
                        }
                        None => {
                            return Some(format!(
                                "Audit trail artifact '{sha}' missing from storage"
                            ))
                        }
                    }
                }
            }
        }
        None
    }

    pub fn verify_research_readiness(
        &self,
        candidate_id: &str,
        audit_resolver: Option<AuditResolver>,
    ) -> Result<crate::domain::ResearchReadiness, String> {
        let mut cand = self
            .get_candidate(candidate_id)
            .ok_or_else(|| format!("Candidate '{candidate_id}' not found"))?;

        let snapshot_text = cand
            .research
            .primary_source
            .as_ref()
            .and_then(|ps| ps.snapshot_ref.as_deref())
            .and_then(|ref_str| self.read_snapshot_text(ref_str));

        let secondary_reader = |hash: &str| self.read_snapshot_text(hash);
        let audit_reader = |hash: &str| {
            if let Some(res) = audit_resolver {
                if let Some(content) = res(hash) {
                    return Some(content);
                }
            }
            self.read_snapshot_text(hash)
        };
        let asset_reader = |hash: &str| self.read_snapshot_bytes(hash);

        let readiness = crate::research_readiness::evaluate_readiness_full(
            cand.research.primary_source.as_ref(),
            snapshot_text.as_deref(),
            &cand.research.claims,
            &cand.research.visual_evidence,
            cand.content_origin,
            Some(&secondary_reader),
            Some(&audit_reader),
            Some(&asset_reader),
        );

        cand.research.research_readiness = Some(readiness.clone());
        let _ = self.storage.save_candidates(std::slice::from_ref(&cand));
        Ok(readiness)
    }
}
