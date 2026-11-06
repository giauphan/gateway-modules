//! In-memory storage adapter implementing `StoragePort`.

use super::merge::{merge_candidate_entry, trim_candidate_cache};
use crate::domain::{HandoffReservation, ModuleConfig, NormalizedCandidate, ReservationStatus};
use crate::ports::StoragePort;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

#[derive(Default)]
pub struct InMemoryStorageAdapter {
    pub(crate) config: RwLock<ModuleConfig>,
    pub(crate) consumed: RwLock<HashSet<String>>,
    pub(crate) cached_candidates: RwLock<Vec<NormalizedCandidate>>,
    pub(crate) reservations: RwLock<HashMap<String, HandoffReservation>>,
}

impl InMemoryStorageAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: ModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            consumed: RwLock::new(HashSet::new()),
            cached_candidates: RwLock::new(Vec::new()),
            reservations: RwLock::new(HashMap::new()),
        }
    }
}

impl StoragePort for InMemoryStorageAdapter {
    fn is_consumed(&self, candidate_id: &str) -> bool {
        if self
            .consumed
            .read()
            .map(|g| g.contains(candidate_id))
            .unwrap_or(false)
        {
            return true;
        }
        if let Ok(guard) = self.reservations.read() {
            if let Some(res) = guard.get(candidate_id) {
                if res.status == ReservationStatus::Published {
                    return true;
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(1793618000);
                if res.status == ReservationStatus::Reserved && now < res.expires_at {
                    return true;
                }
            }
        }
        false
    }

    fn mark_consumed(&self, candidate_ids: &[String]) -> Result<usize, String> {
        let mut count = 0;
        let mut guard = self
            .consumed
            .write()
            .map_err(|e| format!("Lock error: {e}"))?;
        for id in candidate_ids {
            if guard.insert(id.clone()) {
                count += 1;
            }
        }
        Ok(count)
    }

    fn consumed_count(&self) -> usize {
        self.consumed.read().map(|g| g.len()).unwrap_or(0)
    }

    fn load_config(&self) -> ModuleConfig {
        self.config.read().map(|g| g.clone()).unwrap_or_default()
    }

    fn save_config(&self, config: &ModuleConfig) -> Result<(), String> {
        let mut guard = self
            .config
            .write()
            .map_err(|e| format!("Lock error: {e}"))?;
        *guard = config.clone();
        Ok(())
    }

    fn upsert_candidates(&self, candidates: &[NormalizedCandidate]) {
        let Ok(mut guard) = self.cached_candidates.write() else {
            return;
        };

        for incoming in candidates {
            if let Some(pos) = guard
                .iter()
                .position(|c| c.candidate_id == incoming.candidate_id)
            {
                merge_candidate_entry(&mut guard[pos], incoming);
            } else {
                guard.push(incoming.clone());
            }
        }

        trim_candidate_cache(&mut guard, 300);
    }

    fn cache_candidates(&self, candidates: &[NormalizedCandidate]) {
        self.upsert_candidates(candidates);
    }

    fn get_cached_candidates(&self) -> Vec<NormalizedCandidate> {
        self.cached_candidates
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    fn reserve_handoff(
        &self,
        candidate_id: &str,
        lease_seconds: u64,
        worker_id: &str,
    ) -> Result<String, String> {
        let mut guard = self
            .reservations
            .write()
            .map_err(|e| format!("Lock error: {e}"))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1793618000);

        if let Some(res) = guard.get(candidate_id) {
            if res.status == ReservationStatus::Published {
                return Err(format!("Candidate '{candidate_id}' is already PUBLISHED"));
            }
            if res.status == ReservationStatus::Reserved && now < res.expires_at {
                return Err(format!(
                    "Candidate '{candidate_id}' is actively reserved by worker '{}' until {}",
                    res.worker_id, res.expires_at
                ));
            }
        }

        if self
            .consumed
            .read()
            .map(|g| g.contains(candidate_id))
            .unwrap_or(false)
        {
            return Err(format!("Candidate '{candidate_id}' is already consumed"));
        }

        let token = format!("claim_{}_{}", uuid::Uuid::new_v4(), candidate_id);
        let token_hash = crate::url::compute_sha256(&token);
        let reservation = HandoffReservation {
            reservation_id: format!("res_{}", uuid::Uuid::new_v4()),
            candidate_id: candidate_id.to_string(),
            token_hash,
            worker_id: worker_id.to_string(),
            created_at: crate::date::current_utc_timestamp(),
            expires_at: now + lease_seconds,
            status: ReservationStatus::Reserved,
        };

        guard.insert(candidate_id.to_string(), reservation);
        Ok(token)
    }

    fn release_reservation(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        let mut guard = self
            .reservations
            .write()
            .map_err(|e| format!("Lock error: {e}"))?;

        let res = guard
            .get(candidate_id)
            .ok_or_else(|| format!("No reservation found for candidate '{candidate_id}'"))?;

        if res.status == ReservationStatus::Published {
            return Err(format!(
                "Candidate '{candidate_id}' is PUBLISHED; published handoffs cannot be released"
            ));
        }

        let token_hash = crate::url::compute_sha256(token);
        if res.token_hash != token_hash {
            return Err(format!(
                "Invalid reservation token for candidate '{candidate_id}'"
            ));
        }

        guard.remove(candidate_id);
        if let Ok(mut c_guard) = self.consumed.write() {
            c_guard.remove(candidate_id);
        }
        Ok(true)
    }

    fn commit_consumed(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        let mut guard = self
            .reservations
            .write()
            .map_err(|e| format!("Lock error: {e}"))?;

        let res = guard
            .get_mut(candidate_id)
            .ok_or_else(|| format!("No reservation found for candidate '{candidate_id}'"))?;

        let token_hash = crate::url::compute_sha256(token);
        if res.token_hash != token_hash {
            return Err(format!(
                "Invalid reservation token for candidate '{candidate_id}'"
            ));
        }

        if res.status == ReservationStatus::Published {
            return Ok(true);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1793618000);

        if now > res.expires_at {
            return Err(format!(
                "Reservation lease for candidate '{candidate_id}' has expired"
            ));
        }

        res.status = ReservationStatus::Published;
        if let Ok(mut c_guard) = self.consumed.write() {
            c_guard.insert(candidate_id.to_string());
        }
        Ok(true)
    }
}
