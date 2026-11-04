//! Storage adapter for persistence and consumed tracking.
//!
//! Implements `StoragePort` with in-memory caching and persistent JSON files.

use crate::domain::{ModuleConfig, NormalizedCandidate, StoryboardEvidence};
use crate::ports::StoragePort;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

#[derive(Default)]
pub struct InMemoryStorageAdapter {
    pub(crate) config: RwLock<ModuleConfig>,
    pub(crate) consumed: RwLock<HashSet<String>>,
    pub(crate) cached_candidates: RwLock<Vec<NormalizedCandidate>>,
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
        }
    }
}

fn restore_preserved_verification(
    target: &mut NormalizedCandidate,
    preserved_fact_check: Option<serde_json::Value>,
    preserved_evidence: StoryboardEvidence,
) {
    if target.independent_fact_check.is_none() {
        target.independent_fact_check = preserved_fact_check;
    }
    if target.evidence.snapshot_sha256.is_none() {
        target.evidence = preserved_evidence;
    }
}

fn merge_candidate_with_evidence(
    existing: &mut NormalizedCandidate,
    incoming: &NormalizedCandidate,
) {
    let preserved_fact_check = existing.independent_fact_check.clone();
    let preserved_evidence = existing.evidence.clone();
    *existing = incoming.clone();
    restore_preserved_verification(existing, preserved_fact_check, preserved_evidence);
}

fn merge_candidate_preserve_research(
    existing: &mut NormalizedCandidate,
    incoming: &NormalizedCandidate,
) {
    let preserved_research = existing.research.clone();
    let preserved_fact_check = existing.independent_fact_check.clone();
    let preserved_evidence = existing.evidence.clone();
    let preserved_scoring = existing.scoring.clone();

    *existing = incoming.clone();
    existing.research = preserved_research;
    restore_preserved_verification(existing, preserved_fact_check, preserved_evidence);
    if existing.scoring.is_none() {
        existing.scoring = preserved_scoring;
    }
}

fn merge_candidate_entry(existing: &mut NormalizedCandidate, incoming: &NormalizedCandidate) {
    let incoming_has_evidence =
        incoming.research.primary_source.is_some() || !incoming.research.claims.is_empty();

    if incoming_has_evidence || existing.research.primary_source.is_none() {
        merge_candidate_with_evidence(existing, incoming);
    } else {
        merge_candidate_preserve_research(existing, incoming);
    }
}

fn trim_candidate_cache(guard: &mut Vec<NormalizedCandidate>, max_size: usize) {
    if guard.len() > max_size {
        let excess = guard.len() - max_size;
        guard.drain(0..excess);
    }
}

impl StoragePort for InMemoryStorageAdapter {
    fn is_consumed(&self, candidate_id: &str) -> bool {
        self.consumed
            .read()
            .map(|g| g.contains(candidate_id))
            .unwrap_or(false)
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
}

pub struct FileStorageAdapter {
    config_path: PathBuf,
    state_dir: PathBuf,
    inner: InMemoryStorageAdapter,
    persist_mutex: Mutex<()>,
}

impl FileStorageAdapter {
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        let base = base_dir.as_ref();
        let config_path = base.join("config.json");
        let state_dir = std::env::var("DAILY_NEWS_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let candidate = base.join(".state");
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
            });

        let initial_config = if config_path.is_file() {
            fs::read_to_string(&config_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            ModuleConfig::default()
        };

        let consumed_path = state_dir.join("consumed.json");
        let initial_consumed: HashSet<String> = if consumed_path.is_file() {
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

        Self {
            config_path,
            state_dir,
            inner: InMemoryStorageAdapter {
                config: RwLock::new(initial_config),
                consumed: RwLock::new(initial_consumed),
                cached_candidates: RwLock::new(initial_candidates),
            },
            persist_mutex: Mutex::new(()),
        }
    }

    fn persist_consumed(&self) {
        let consumed_path = self.state_dir.join("consumed.json");
        if let Ok(guard) = self.inner.consumed.read() {
            let list: Vec<String> = guard.iter().cloned().collect();
            if let Ok(json) = serde_json::to_string_pretty(&list) {
                let _ = fs::write(consumed_path, json);
            }
        }
    }
}

impl StoragePort for FileStorageAdapter {
    fn is_consumed(&self, candidate_id: &str) -> bool {
        self.inner.is_consumed(candidate_id)
    }

    fn mark_consumed(&self, candidate_ids: &[String]) -> Result<usize, String> {
        let count = self.inner.mark_consumed(candidate_ids)?;
        self.persist_consumed();
        Ok(count)
    }

    fn consumed_count(&self) -> usize {
        self.inner.consumed_count()
    }

    fn load_config(&self) -> ModuleConfig {
        self.inner.load_config()
    }

    fn save_config(&self, config: &ModuleConfig) -> Result<(), String> {
        self.inner.save_config(config)?;
        if let Ok(json) = serde_json::to_string_pretty(config) {
            let _ = fs::write(&self.config_path, json);
        }
        Ok(())
    }

    fn upsert_candidates(&self, candidates: &[NormalizedCandidate]) {
        let _ = self.save_candidates(candidates);
    }

    fn cache_candidates(&self, candidates: &[NormalizedCandidate]) {
        let _ = self.save_candidates(candidates);
    }

    fn save_candidates(&self, candidates: &[NormalizedCandidate]) -> Result<usize, String> {
        let _lock = match self.persist_mutex.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        // 1. Transactional isolation: build proposed candidate list without mutating RAM yet
        let mut proposed = self.inner.get_cached_candidates();
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

        // 2. Write to disk first atomically
        let _ = fs::create_dir_all(&self.state_dir);
        let candidates_path = self.state_dir.join("candidates.json");
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

        // 3. ONLY after disk write & atomic rename succeed, commit to RAM cache!
        if let Ok(mut guard) = self.inner.cached_candidates.write() {
            *guard = proposed;
        }

        Ok(candidates.len())
    }

    fn get_cached_candidates(&self) -> Vec<NormalizedCandidate> {
        self.inner.get_cached_candidates()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_adapter_lifecycle() {
        let tmp = std::env::temp_dir().join(format!("test_dn_storage_{}", uuid::Uuid::new_v4()));
        let storage = FileStorageAdapter::new(&tmp);

        assert_eq!(storage.consumed_count(), 0);
        assert!(!storage.is_consumed("cand_1"));

        let added = storage
            .mark_consumed(&["cand_1".to_string(), "cand_2".to_string()])
            .unwrap();
        assert_eq!(added, 2);
        assert!(storage.is_consumed("cand_1"));
        assert!(storage.is_consumed("cand_2"));
        assert_eq!(storage.consumed_count(), 2);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_candidate_persistent_upsert_and_restart_recovery() {
        let tmp = std::env::temp_dir().join(format!(
            "test_dn_storage_candidates_{}",
            uuid::Uuid::new_v4()
        ));
        {
            let storage = FileStorageAdapter::new(&tmp);
            assert_eq!(storage.get_cached_candidates().len(), 0);

            // 1. Insert 10 distinct candidates
            let mut candidates = Vec::new();
            for i in 1..=10 {
                let c = NormalizedCandidate {
                    candidate_id: format!("cand_{i}"),
                    title: format!("Candidate Title {i}"),
                    canonical_url: format!("https://example.com/article/{i}"),
                    ..Default::default()
                };
                candidates.push(c);
            }
            storage.cache_candidates(&candidates);
            assert_eq!(storage.get_cached_candidates().len(), 10);

            // 2. Enrich candidate 3 and upsert
            let mut cand_3 = storage.get_candidate("cand_3").expect("Must find cand_3");
            cand_3.body = Some("Enriched technical content for candidate 3".to_string());
            cand_3.research.hook_origin = Some("verified_hook".to_string());
            storage.upsert_candidates(&[cand_3]);

            // All 10 still present, cand_3 has enriched body
            let current = storage.get_cached_candidates();
            assert_eq!(current.len(), 10);
            let updated_3 = storage.get_candidate("cand_3").unwrap();
            assert_eq!(
                updated_3.body.as_deref(),
                Some("Enriched technical content for candidate 3")
            );
        }

        // 3. Restart module: instantiate new adapter from same state directory
        {
            let restarted_storage = FileStorageAdapter::new(&tmp);
            let recovered = restarted_storage.get_cached_candidates();
            assert_eq!(
                recovered.len(),
                10,
                "All 10 candidates must survive restart"
            );
            let cand_3 = restarted_storage
                .get_candidate("cand_3")
                .expect("cand_3 must survive");
            assert_eq!(
                cand_3.body.as_deref(),
                Some("Enriched technical content for candidate 3")
            );
            assert_eq!(
                cand_3.research.hook_origin.as_deref(),
                Some("verified_hook")
            );
        }

        let _ = fs::remove_dir_all(tmp);
    }

    #[test]
    fn test_save_candidates_error_propagation_on_write_failure() {
        let tmp =
            std::env::temp_dir().join(format!("test_dn_storage_err_{}", uuid::Uuid::new_v4()));
        let storage = FileStorageAdapter::new(&tmp);

        // Turn state_dir into a file to induce an I/O error during tmp file creation
        let _ = fs::remove_dir_all(&storage.state_dir);
        let _ = fs::write(&storage.state_dir, b"blocker");

        let c = NormalizedCandidate {
            candidate_id: "cand_err_test".to_string(),
            ..Default::default()
        };
        let res = storage.save_candidates(&[c]);
        assert!(res.is_err(), "Must propagate I/O error to caller");
        assert!(res.unwrap_err().contains("Failed to write candidate state"));

        let _ = fs::remove_file(&storage.state_dir);
        let _ = fs::remove_dir_all(&tmp);
    }
}
