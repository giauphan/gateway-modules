//! Production handoff, reservation, and publish idempotency use cases for DailyNewsService.

use super::DailyNewsService;
use crate::date::parse_iso_to_epoch;
use crate::domain::{ContentOrigin, IdempotencyRecord, ProductionHandoff, ReadinessStatus};

impl DailyNewsService {
    pub fn verify_production_handoff(
        &self,
        candidate_id: &str,
    ) -> Result<ProductionHandoff, (String, String)> {
        // 1. Check if already consumed / claimed
        match self.storage.try_is_consumed(candidate_id) {
            Ok(true) => {
                return Err((
                    "ALREADY_CONSUMED".to_string(),
                    format!("Candidate '{candidate_id}' has already been consumed and claimed for production"),
                ));
            }
            Err(e) => {
                return Err((
                    "STORAGE_ERROR".to_string(),
                    format!("Storage error: {e}"),
                ));
            }
            Ok(false) => {}
        }

        let mut cand = self.get_candidate(candidate_id).ok_or_else(|| {
            (
                "CANDIDATE_NOT_FOUND".to_string(),
                format!("Candidate '{candidate_id}' not found"),
            )
        })?;

        // 2. Freshness check against sliding max_age_days and freshness_window_hours
        let cfg = self.storage.load_config();
        let ref_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1793618000);
        let max_age_secs = (cfg.max_age_days as u64) * 86400;
        let window_secs = if cfg.freshness_window_hours > 0.0 {
            ((cfg.freshness_window_hours * 3600.0) as u64).min(max_age_secs)
        } else {
            max_age_secs
        };
        let cutoff_epoch = ref_epoch.saturating_sub(window_secs);

        let effective_ts = cand.updated_at.as_deref().unwrap_or(&cand.published_at_utc);
        let cand_epoch = parse_iso_to_epoch(effective_ts).unwrap_or(0);
        if cand_epoch < cutoff_epoch && (cfg.require_latest_news || !cand.is_evergreen) {
            return Err((
                "STALE_CANDIDATE".to_string(),
                format!(
                    "Candidate '{candidate_id}' timestamp '{effective_ts}' is older than cutoff max age ({} days / {} hours window)",
                    cfg.max_age_days, cfg.freshness_window_hours
                ),
            ));
        }

        // 3. For feed_observed articles, require verified primary source publication freshness
        if cand.published_at_kind == "feed_observed" && cand.content_origin == ContentOrigin::Article {
            let primary_pub = cand
                .primary_source
                .as_ref()
                .and_then(|p| p.published_at.as_deref())
                .and_then(|ts| parse_iso_to_epoch(ts).ok());
            match primary_pub {
                Some(ep) if ep >= cutoff_epoch => {},
                Some(_) => {
                    return Err((
                        "STALE_CANDIDATE".to_string(),
                        format!("Candidate '{candidate_id}' verified original article publication time is stale"),
                    ));
                }
                None => {
                    return Err((
                        "UNVERIFIED_PUBLICATION_DATE".to_string(),
                        format!("Candidate '{candidate_id}' feed-observed article lacks verified primary source publication date for production handoff"),
                    ));
                }
            }
        }

        // 4. Re-evaluate readiness
        let ps = cand.research.primary_source.as_ref();
        let snapshot = ps.and_then(|p| {
            p.snapshot_ref
                .as_deref()
                .and_then(|r| self.read_snapshot_text(r))
                .or_else(|| {
                    p.snapshot_sha256
                        .as_deref()
                        .and_then(|h| self.read_snapshot_text(h))
                })
        });

        let text_reader = |h: &str| self.read_snapshot_text(h);
        let asset_reader = |h: &str| self.read_snapshot_bytes(h);

        let fresh = crate::research_readiness::evaluate_readiness_full(
            ps,
            snapshot.as_deref(),
            &cand.research.claims,
            &cand.research.visual_evidence,
            cand.content_origin,
            Some(&text_reader),
            Some(&text_reader),
            Some(&asset_reader),
        );

        let changed = cand
            .research
            .research_readiness
            .as_ref()
            .map(|r| r.status)
            != Some(fresh.status);
        let status = fresh.status;
        cand.research.research_readiness = Some(fresh);

        if changed {
            if let Err(e) = self.storage.save_candidates(std::slice::from_ref(&cand)) {
                eprintln!("[daily-news] Warning: Failed to persist verified candidate in verify_production_handoff: {e}");
            }
        }

        if status != ReadinessStatus::VideoReady {
            return Err((
                "HANDOFF_REJECTED".to_string(),
                format!(
                    "Candidate '{candidate_id}' readiness status is '{}'; VIDEO_READY required for production handoff",
                    status.as_str()
                ),
            ));
        }

        // 5. Atomic reservation / claim with lease token
        let lease_seconds = 3600u64;
        let token = self
            .storage
            .reserve_handoff(candidate_id, lease_seconds, "production_worker")
            .map_err(|e| ("ALREADY_CONSUMED".to_string(), e))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1793618000);

        Ok(ProductionHandoff {
            candidate: cand,
            reservation_token: token,
            lease_seconds,
            expires_at: now + lease_seconds,
        })
    }

    pub fn release_handoff(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        self.storage.release_reservation(candidate_id, token)
    }

    pub fn commit_handoff_with_idempotency(
        &self,
        candidate_id: &str,
        token: &str,
        idempotency_key: Option<&str>,
    ) -> Result<bool, String> {
        self.storage
            .commit_consumed_with_idempotency(candidate_id, token, idempotency_key)
    }

    pub fn commit_handoff(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        self.commit_handoff_with_idempotency(candidate_id, token, None)
    }

    pub fn register_publish_idempotency(
        &self,
        key: &str,
        candidate_id: &str,
        action: &str,
    ) -> Result<bool, String> {
        self.storage.register_idempotency_key(key, candidate_id, action)
    }

    pub fn check_publish_idempotency(
        &self,
        key: &str,
    ) -> Result<Option<IdempotencyRecord>, String> {
        self.storage.check_idempotency_key(key)
    }
}
