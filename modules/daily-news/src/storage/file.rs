//! File-backed transactional storage adapter with inter-process kernel flock.

use super::in_memory::InMemoryStorageAdapter;
use super::lock::ProcessFileLock;
use super::merge::{merge_candidate_entry, trim_candidate_cache};
use crate::domain::{HandoffReservation, ModuleConfig, NormalizedCandidate, ReservationStatus};
use crate::ports::StoragePort;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

pub struct FileStorageAdapter {
    config_path: PathBuf,
    pub(crate) state_dir: PathBuf,
    inner: InMemoryStorageAdapter,
    persist_mutex: Mutex<()>,
}

pub fn resolve_state_dir(base_dir: &Path) -> PathBuf {
    std::env::var("DAILY_NEWS_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let candidate = base_dir.join(".state");
            let is_writable = {
                let _ = fs::create_dir_all(&candidate);
                let test_file = candidate.join(format!(".wt_{}", uuid::Uuid::new_v4()));
                if fs::write(&test_file, b"ok").is_ok() {
                    let _ = fs::remove_file(&test_file);
                    true
                } else {
                    false
                }
            };
            if is_writable {
                candidate
            } else {
                let fallback = std::env::temp_dir().join("daily-news").join(".state");
                let _ = fs::create_dir_all(&fallback);
                fallback
            }
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitJournalEntry {
    pub candidate_id: String,
    pub published_reservation: HandoffReservation,
    pub timestamp: u64,
}

fn reconcile_transaction_journal(state_dir: &Path) {
    let journal_path = state_dir.join("commit_tx.journal");
    if !journal_path.is_file() {
        return;
    }

    let Ok(content) = fs::read_to_string(&journal_path) else {
        return;
    };
    let Ok(entry) = serde_json::from_str::<CommitJournalEntry>(&content) else {
        return;
    };

    let reservations_path = state_dir.join("reservations.json");
    let mut reservations: HashMap<String, HandoffReservation> = if reservations_path.is_file() {
        fs::read_to_string(&reservations_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let cid = entry.candidate_id.clone();
    reservations.insert(cid.clone(), entry.published_reservation);

    let unique_id = uuid::Uuid::new_v4();
    let tmp_res = state_dir.join(format!("reservations.{unique_id}.tmp"));
    let Ok(json) = serde_json::to_string_pretty(&reservations) else {
        return;
    };
    if fs::write(&tmp_res, json).is_err() {
        let _ = fs::remove_file(&tmp_res);
        return;
    }
    if fs::rename(&tmp_res, &reservations_path).is_err() {
        let _ = fs::remove_file(&tmp_res);
        return;
    }

    let consumed_path = state_dir.join("consumed.json");
    let mut consumed_set: HashSet<String> = if consumed_path.is_file() {
        fs::read_to_string(&consumed_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    consumed_set.insert(cid);

    let tmp_c = state_dir.join(format!("consumed.{unique_id}.tmp"));
    let Ok(cj) = serde_json::to_string_pretty(&consumed_set) else {
        return;
    };
    if fs::write(&tmp_c, cj).is_err() {
        let _ = fs::remove_file(&tmp_c);
        return;
    }
    if fs::rename(&tmp_c, &consumed_path).is_err() {
        let _ = fs::remove_file(&tmp_c);
        return;
    }

    // Both files updated atomically; only now remove journal
    let _ = fs::remove_file(&journal_path);
}

impl FileStorageAdapter {
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        let base = base_dir.as_ref();
        let state_dir = resolve_state_dir(base);
        let _ = fs::create_dir_all(&state_dir);
        let _process_lock = ProcessFileLock::acquire(&state_dir.join(".state.lock"));
        reconcile_transaction_journal(&state_dir);
        let config_path = state_dir.join("config.json");
        let legacy_config_path = base.join("config.json");

        let initial_config = if config_path.is_file() {
            fs::read_to_string(&config_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else if legacy_config_path.is_file() {
            fs::read_to_string(&legacy_config_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            ModuleConfig::default()
        };

        let consumed_path = state_dir.join("consumed.json");
        let mut initial_consumed: HashSet<String> = if consumed_path.is_file() {
            fs::read_to_string(&consumed_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            HashSet::new()
        };

        let candidates_path = state_dir.join("candidates.json");
        let initial_candidates: Vec<NormalizedCandidate> = if candidates_path.is_file() {
            fs::read_to_string(&candidates_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let reservations_path = state_dir.join("reservations.json");
        let initial_reservations: HashMap<String, HandoffReservation> = if reservations_path.is_file() {
            fs::read_to_string(&reservations_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        // State reconciliation: any candidate marked Published in reservations.json
        // is authoritatively consumed, preventing desync across unexpected restarts.
        for (cid, res) in &initial_reservations {
            if res.status == ReservationStatus::Published {
                initial_consumed.insert(cid.clone());
            }
        }

        Self {
            config_path,
            state_dir,
            inner: InMemoryStorageAdapter {
                config: RwLock::new(initial_config),
                consumed: RwLock::new(initial_consumed),
                cached_candidates: RwLock::new(initial_candidates),
                reservations: RwLock::new(initial_reservations),
            },
            persist_mutex: Mutex::new(()),
        }
    }

    fn with_lock<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce() -> Result<R, String>,
    {
        let _thread_lock = self
            .persist_mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = fs::create_dir_all(&self.state_dir);
        let _process_lock = ProcessFileLock::acquire(&self.state_dir.join(".state.lock"))?;
        reconcile_transaction_journal(&self.state_dir);
        f()
    }

    fn load_reservations_map(&self) -> HashMap<String, HandoffReservation> {
        let reservations_path = self.state_dir.join("reservations.json");
        if reservations_path.is_file() {
            fs::read_to_string(&reservations_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            self.inner.reservations.read().map(|g| g.clone()).unwrap_or_default()
        }
    }
}

fn is_consumed_in_file(path: &Path, candidate_id: &str) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<HashSet<String>>(&content)
        .map(|set| set.contains(candidate_id))
        .unwrap_or(false)
}

fn is_reserved_in_file(path: &Path, candidate_id: &str) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(map) = serde_json::from_str::<HashMap<String, HandoffReservation>>(&content) else {
        return false;
    };
    let Some(res) = map.get(candidate_id) else {
        return false;
    };
    if res.status == ReservationStatus::Published {
        return true;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1793618000);
    res.status == ReservationStatus::Reserved && now < res.expires_at
}

fn validate_reservation_release(
    res: &HandoffReservation,
    candidate_id: &str,
    token: &str,
) -> Result<(), String> {
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
    Ok(())
}

fn write_reservations_file(
    state_dir: &Path,
    reservations: &HashMap<String, HandoffReservation>,
) -> Result<(), String> {
    let reservations_path = state_dir.join("reservations.json");
    let unique_id = uuid::Uuid::new_v4();
    let tmp_path = state_dir.join(format!("reservations.{unique_id}.tmp"));
    let json = serde_json::to_string_pretty(reservations)
        .map_err(|e| format!("Failed to serialize reservations: {e}"))?;
    fs::write(&tmp_path, json).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to write reservations to {tmp_path:?}: {e}")
    })?;
    fs::rename(&tmp_path, &reservations_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Failed to finalize reservations to {reservations_path:?}: {e}")
    })
}

fn remove_from_consumed_file(state_dir: &Path, candidate_id: &str) {
    let consumed_path = state_dir.join("consumed.json");
    if !consumed_path.is_file() {
        return;
    }
    let Ok(content) = fs::read_to_string(&consumed_path) else {
        return;
    };
    let Ok(mut list) = serde_json::from_str::<HashSet<String>>(&content) else {
        return;
    };
    if list.remove(candidate_id) {
        let tmp_c = state_dir.join(format!("consumed.{}.tmp", uuid::Uuid::new_v4()));
        if let Ok(cj) = serde_json::to_string_pretty(&list) {
            if fs::write(&tmp_c, cj).is_ok() {
                let _ = fs::rename(&tmp_c, &consumed_path);
            }
        }
    }
}

impl StoragePort for FileStorageAdapter {
    fn is_consumed(&self, candidate_id: &str) -> bool {
        self.inner.is_consumed(candidate_id)
            || is_consumed_in_file(&self.state_dir.join("consumed.json"), candidate_id)
            || is_reserved_in_file(&self.state_dir.join("reservations.json"), candidate_id)
    }

    fn mark_consumed(&self, candidate_ids: &[String]) -> Result<usize, String> {
        self.with_lock(|| {
            let consumed_path = self.state_dir.join("consumed.json");
            let mut proposed = if consumed_path.is_file() {
                fs::read_to_string(&consumed_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
                    .map(|v| v.into_iter().collect::<HashSet<String>>())
                    .unwrap_or_else(|| {
                        self.inner
                            .consumed
                            .read()
                            .map(|g| g.clone())
                            .unwrap_or_default()
                    })
            } else {
                self.inner
                    .consumed
                    .read()
                    .map(|g| g.clone())
                    .unwrap_or_default()
            };

            let mut count = 0;
            for id in candidate_ids {
                if proposed.insert(id.clone()) {
                    count += 1;
                }
            }

            if count == 0 {
                return Ok(0);
            }

            let unique_id = uuid::Uuid::new_v4();
            let tmp_path = self.state_dir.join(format!("consumed.{unique_id}.tmp"));
            let list: Vec<String> = proposed.iter().cloned().collect();
            let json = serde_json::to_string_pretty(&list)
                .map_err(|e| format!("Failed to serialize consumed IDs: {e}"))?;

            fs::write(&tmp_path, json).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("Failed to write consumed IDs to {tmp_path:?}: {e}")
            })?;

            fs::rename(&tmp_path, &consumed_path).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("Failed to finalize consumed state to {consumed_path:?}: {e}")
            })?;

            let mut guard = self
                .inner
                .consumed
                .write()
                .map_err(|e| format!("Lock error updating consumed IDs: {e}"))?;
            *guard = proposed;

            Ok(count)
        })
    }

    fn reserve_handoff(
        &self,
        candidate_id: &str,
        lease_seconds: u64,
        worker_id: &str,
    ) -> Result<String, String> {
        self.with_lock(|| {
            let consumed_path = self.state_dir.join("consumed.json");
            let consumed_set: HashSet<String> = if consumed_path.is_file() {
                fs::read_to_string(&consumed_path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default()
            } else {
                self.inner.consumed.read().map(|g| g.clone()).unwrap_or_default()
            };

            let mut reservations = self.load_reservations_map();
            let now = super::reservation::current_epoch_seconds();
            let is_consumed = consumed_set.contains(candidate_id);

            super::reservation::check_reservation_claimable(reservations.get(candidate_id), candidate_id, is_consumed, now)?;
            let (token, reservation) = super::reservation::create_handoff_reservation(candidate_id, worker_id, lease_seconds, now);
            reservations.insert(candidate_id.to_string(), reservation);

            write_reservations_file(&self.state_dir, &reservations)?;

            if let Ok(mut guard) = self.inner.reservations.write() {
                *guard = reservations;
            }

            Ok(token)
        })
    }

    fn release_reservation(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        self.with_lock(|| {
            let mut reservations = self.load_reservations_map();
            let res = reservations
                .get(candidate_id)
                .ok_or_else(|| format!("No reservation found for candidate '{candidate_id}'"))?;

            validate_reservation_release(res, candidate_id, token)?;
            reservations.remove(candidate_id);

            write_reservations_file(&self.state_dir, &reservations)?;
            remove_from_consumed_file(&self.state_dir, candidate_id);

            if let Ok(mut guard) = self.inner.reservations.write() {
                *guard = reservations;
            }
            if let Ok(mut guard) = self.inner.consumed.write() {
                guard.remove(candidate_id);
            }

            Ok(true)
        })
    }

    fn commit_consumed(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        self.with_lock(|| {
            let mut reservations = self.load_reservations_map();
            let res = reservations
                .get_mut(candidate_id)
                .ok_or_else(|| format!("No reservation found for candidate '{candidate_id}'"))?;

            let now = super::reservation::current_epoch_seconds();
            if super::reservation::validate_reservation_commit(res, candidate_id, token, now)? {
                return Ok(true);
            }

            res.status = ReservationStatus::Published;

            // Step 1: Write journal entry atomically (Point of Commit)
            let journal_entry = CommitJournalEntry {
                candidate_id: candidate_id.to_string(),
                published_reservation: res.clone(),
                timestamp: now,
            };
            let unique_id = uuid::Uuid::new_v4();
            let journal_path = self.state_dir.join("commit_tx.journal");
            let tmp_journal = self.state_dir.join(format!("commit.{unique_id}.tmp"));
            let journal_json = serde_json::to_string_pretty(&journal_entry)
                .map_err(|e| format!("Failed to serialize commit journal: {e}"))?;
            fs::write(&tmp_journal, journal_json).map_err(|e| {
                let _ = fs::remove_file(&tmp_journal);
                format!("Failed to write commit journal: {e}")
            })?;
            fs::rename(&tmp_journal, &journal_path).map_err(|e| {
                let _ = fs::remove_file(&tmp_journal);
                format!("Failed to finalize commit journal: {e}")
            })?;

            // Step 2: Write reservations.json
            write_reservations_file(&self.state_dir, &reservations)?;

            // Step 3: Write consumed.json
            let consumed_path = self.state_dir.join("consumed.json");
            let mut consumed_set: HashSet<String> = if consumed_path.is_file() {
                fs::read_to_string(&consumed_path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default()
            } else {
                self.inner.consumed.read().map(|g| g.clone()).unwrap_or_default()
            };
            consumed_set.insert(candidate_id.to_string());

            let tmp_c = self.state_dir.join(format!("consumed.{}.tmp", uuid::Uuid::new_v4()));
            let cj = serde_json::to_string_pretty(&consumed_set)
                .map_err(|e| format!("Failed to serialize consumed set: {e}"))?;
            fs::write(&tmp_c, cj).map_err(|e| {
                let _ = fs::remove_file(&tmp_c);
                format!("Failed to write consumed set to {tmp_c:?}: {e}")
            })?;
            fs::rename(&tmp_c, &consumed_path).map_err(|e| {
                let _ = fs::remove_file(&tmp_c);
                format!("Failed to finalize consumed set to {consumed_path:?}: {e}")
            })?;

            // Step 4: Transaction completed; remove journal
            let _ = fs::remove_file(&journal_path);

            if let Ok(mut guard) = self.inner.reservations.write() {
                *guard = reservations;
            }
            if let Ok(mut guard) = self.inner.consumed.write() {
                guard.insert(candidate_id.to_string());
            }

            Ok(true)
        })
    }

    fn consumed_count(&self) -> usize {
        self.inner.consumed_count()
    }

    fn load_config(&self) -> ModuleConfig {
        self.inner.load_config()
    }

    fn save_config(&self, config: &ModuleConfig) -> Result<(), String> {
        self.with_lock(|| {
            let json = serde_json::to_string_pretty(config)
                .map_err(|e| format!("Failed to serialize config: {e}"))?;
            let unique_id = uuid::Uuid::new_v4();
            let tmp_path = self.state_dir.join(format!("config.{unique_id}.tmp"));

            fs::write(&tmp_path, json).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("Failed to write config state to {tmp_path:?}: {e}")
            })?;

            fs::rename(&tmp_path, &self.config_path).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("Failed to finalize config state to {:?}: {e}", self.config_path)
            })?;

            self.inner.save_config(config)?;
            Ok(())
        })
    }

    fn upsert_candidates(&self, candidates: &[NormalizedCandidate]) {
        let _ = self.save_candidates(candidates);
    }

    fn cache_candidates(&self, candidates: &[NormalizedCandidate]) {
        let _ = self.save_candidates(candidates);
    }

    fn save_candidates(&self, candidates: &[NormalizedCandidate]) -> Result<usize, String> {
        self.with_lock(|| {
            let candidates_path = self.state_dir.join("candidates.json");
            let mut proposed = if candidates_path.is_file() {
                fs::read_to_string(&candidates_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<Vec<NormalizedCandidate>>(&s).ok())
                    .unwrap_or_else(|| self.inner.get_cached_candidates())
            } else {
                self.inner.get_cached_candidates()
            };

            for incoming in candidates {
                if let Some(pos) = proposed
                    .iter()
                    .position(|c| c.candidate_id == incoming.candidate_id)
                {
                    merge_candidate_entry(&mut proposed[pos], incoming);
                } else {
                    proposed.push(incoming.clone());
                }
            }
            trim_candidate_cache(&mut proposed, 300);

            let unique_id = uuid::Uuid::new_v4();
            let tmp_path = self.state_dir.join(format!("candidates.{unique_id}.tmp"));
            let json = serde_json::to_string_pretty(&proposed)
                .map_err(|e| format!("Failed to serialize candidates: {e}"))?;

            fs::write(&tmp_path, json).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("Failed to write candidate state to {tmp_path:?}: {e}")
            })?;

            fs::rename(&tmp_path, &candidates_path).map_err(|e| {
                let _ = fs::remove_file(&tmp_path);
                format!("Failed to finalize candidate state to {candidates_path:?}: {e}")
            })?;

            if let Ok(mut guard) = self.inner.cached_candidates.write() {
                *guard = proposed;
            }

            Ok(candidates.len())
        })
    }

    fn get_cached_candidates(&self) -> Vec<NormalizedCandidate> {
        let cached = self.inner.get_cached_candidates();
        if !cached.is_empty() {
            return cached;
        }
        let candidates_path = self.state_dir.join("candidates.json");
        if candidates_path.is_file() {
            if let Ok(content) = fs::read_to_string(&candidates_path) {
                if let Ok(list) = serde_json::from_str::<Vec<NormalizedCandidate>>(&content) {
                    if let Ok(mut guard) = self.inner.cached_candidates.write() {
                        *guard = list.clone();
                    }
                    return list;
                }
            }
        }
        cached
    }
}
