//! Storage adapter for persistence and consumed tracking.
//!
//! Implements `StoragePort` with in-memory caching and persistent JSON files.

use crate::domain::{ModuleConfig, NormalizedCandidate};
use crate::ports::StoragePort;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

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

    fn cache_candidates(&self, candidates: &[NormalizedCandidate]) {
        if let Ok(mut guard) = self.cached_candidates.write() {
            *guard = candidates.to_vec();
        }
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
}

impl FileStorageAdapter {
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        let base = base_dir.as_ref();
        let config_path = base.join("config.json");
        let state_dir = base.join(".state");
        let _ = fs::create_dir_all(&state_dir);

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

        Self {
            config_path,
            state_dir,
            inner: InMemoryStorageAdapter {
                config: RwLock::new(initial_config),
                consumed: RwLock::new(initial_consumed),
                cached_candidates: RwLock::new(Vec::new()),
            },
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

    fn cache_candidates(&self, candidates: &[NormalizedCandidate]) {
        self.inner.cache_candidates(candidates);
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

        let _ = fs::remove_dir_all(tmp);
    }
}
