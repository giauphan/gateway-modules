//! File and In-Memory Storage Adapter.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.
//! Implements RuleRepositoryPort and MetricsStoragePort with atomic file replacement.

use crate::domain::{FlowMetrics, Rule};
use crate::ports::{MetricsStoragePort, RuleRepositoryPort};
use crate::rules_engine::RulesEngine;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct FileStorageAdapter {
    base_dir: PathBuf,
    rules_cache: RwLock<Vec<Rule>>,
    whitelist_cache: RwLock<Vec<String>>,
    metrics_cache: RwLock<FlowMetrics>,
}

impl FileStorageAdapter {
    pub fn new(base_dir: &Path) -> Self {
        let state_dir = base_dir.join(".state");
        let _ = fs::create_dir_all(&state_dir);

        let adapter = Self {
            base_dir: base_dir.to_path_buf(),
            rules_cache: RwLock::new(RulesEngine::builtin_rules()),
            whitelist_cache: RwLock::new(Vec::new()),
            metrics_cache: RwLock::new(FlowMetrics::new(3.5)),
        };

        adapter.load_persisted_state();
        adapter
    }

    fn rules_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("rules.json")
    }

    fn whitelist_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("whitelist.json")
    }

    fn metrics_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("metrics.json")
    }

    fn atomic_write_file(&self, target_path: &Path, content: &str) -> Result<(), String> {
        let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let temp_filename = format!(
            ".{}_{}_{}.tmp",
            target_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("state"),
            pid,
            counter
        );
        let temp_path = parent.join(temp_filename);

        fs::write(&temp_path, content).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            format!("Failed to write temporary state file {temp_path:?}: {e}")
        })?;

        fs::rename(&temp_path, target_path).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            format!("Failed to atomically rename {temp_path:?} to {target_path:?}: {e}")
        })?;

        Ok(())
    }

    fn load_persisted_state(&self) {
        if let Ok(data) = fs::read_to_string(self.rules_file()) {
            if let Ok(loaded) = serde_json::from_str::<Vec<Rule>>(&data) {
                if let Ok(mut lock) = self.rules_cache.write() {
                    *lock = loaded;
                }
            }
        }

        if let Ok(data) = fs::read_to_string(self.whitelist_file()) {
            if let Ok(loaded) = serde_json::from_str::<Vec<String>>(&data) {
                if let Ok(mut lock) = self.whitelist_cache.write() {
                    *lock = loaded;
                }
            }
        }

        if let Ok(data) = fs::read_to_string(self.metrics_file()) {
            if let Ok(loaded) = serde_json::from_str::<FlowMetrics>(&data) {
                if let Ok(mut lock) = self.metrics_cache.write() {
                    *lock = loaded;
                }
            }
        }
    }
}

impl RuleRepositoryPort for FileStorageAdapter {
    fn load_rules(&self) -> Vec<Rule> {
        self.rules_cache
            .read()
            .map(|r| r.clone())
            .unwrap_or_else(|_| RulesEngine::builtin_rules())
    }

    fn save_rules(&self, rules: &[Rule]) -> Result<(), String> {
        let mut lock = self
            .rules_cache
            .write()
            .map_err(|e| format!("Lock acquisition error: {e}"))?;
        let json = serde_json::to_string_pretty(rules).map_err(|e| e.to_string())?;
        self.atomic_write_file(&self.rules_file(), &json)?;
        *lock = rules.to_vec();
        Ok(())
    }

    fn load_whitelist(&self) -> Vec<String> {
        self.whitelist_cache
            .read()
            .map(|w| w.clone())
            .unwrap_or_default()
    }

    fn save_whitelist(&self, whitelist: &[String]) -> Result<(), String> {
        let mut lock = self
            .whitelist_cache
            .write()
            .map_err(|e| format!("Lock acquisition error: {e}"))?;
        let json = serde_json::to_string_pretty(whitelist).map_err(|e| e.to_string())?;
        self.atomic_write_file(&self.whitelist_file(), &json)?;
        *lock = whitelist.to_vec();
        Ok(())
    }
}

impl MetricsStoragePort for FileStorageAdapter {
    fn load_metrics(&self) -> FlowMetrics {
        self.metrics_cache
            .read()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    fn save_metrics(&self, metrics: &FlowMetrics) -> Result<(), String> {
        let mut lock = self
            .metrics_cache
            .write()
            .map_err(|e| format!("Lock acquisition error: {e}"))?;
        let json = serde_json::to_string_pretty(metrics).map_err(|e| e.to_string())?;
        self.atomic_write_file(&self.metrics_file(), &json)?;
        *lock = metrics.clone();
        Ok(())
    }
}
