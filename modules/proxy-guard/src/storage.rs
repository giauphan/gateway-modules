//! File and In-Memory Storage Adapter.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.
//! Implements RuleRepositoryPort and MetricsStoragePort with atomic file replacement.

use crate::domain::{FlowMetrics, Rule, UpstreamProxy};
use crate::ports::{MetricsStoragePort, ProxyPoolStoragePort, RuleRepositoryPort};
use crate::rules_engine::RulesEngine;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct FileStorageAdapter {
    base_dir: PathBuf,
    rules_cache: RwLock<Vec<Rule>>,
    whitelist_cache: RwLock<Vec<String>>,
    metrics_cache: RwLock<FlowMetrics>,
    proxies_cache: RwLock<Vec<UpstreamProxy>>,
    rules_corrupted: AtomicBool,
    whitelist_corrupted: AtomicBool,
    metrics_corrupted: AtomicBool,
    proxies_corrupted: AtomicBool,
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
            proxies_cache: RwLock::new(Vec::new()),
            rules_corrupted: AtomicBool::new(false),
            whitelist_corrupted: AtomicBool::new(false),
            metrics_corrupted: AtomicBool::new(false),
            proxies_corrupted: AtomicBool::new(false),
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

    fn proxies_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("proxies.json")
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

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);

        let mut file = options
            .open(&temp_path)
            .map_err(|e| format!("Failed to create temporary state file {temp_path:?}: {e}"))?;

        if let Err(e) = file.write_all(content.as_bytes()) {
            if let Err(cleanup_err) = fs::remove_file(&temp_path) {
                if cleanup_err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[proxy-guard] Warning: Failed to remove temp file {temp_path:?}: {cleanup_err}");
                }
            }
            return Err(format!(
                "Failed to write temporary state file {temp_path:?}: {e}"
            ));
        }

        if let Err(e) = file.sync_all() {
            if let Err(cleanup_err) = fs::remove_file(&temp_path) {
                if cleanup_err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[proxy-guard] Warning: Failed to remove temp file {temp_path:?}: {cleanup_err}");
                }
            }
            return Err(format!(
                "Failed to sync temporary state file {temp_path:?}: {e}"
            ));
        }
        drop(file);

        fs::rename(&temp_path, target_path).map_err(|e| {
            if let Err(cleanup_err) = fs::remove_file(&temp_path) {
                if cleanup_err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[proxy-guard] Warning: Failed to remove temp file {temp_path:?}: {cleanup_err}");
                }
            }
            format!("Failed to atomically rename {temp_path:?} to {target_path:?}: {e}")
        })?;

        #[cfg(unix)]
        if let Err(e) = fs::set_permissions(target_path, fs::Permissions::from_mode(0o600)) {
            eprintln!(
                "[proxy-guard] Warning: Failed to set private permissions on {target_path:?}: {e}"
            );
        }

        Ok(())
    }

    fn load_persisted_state(&self) {
        let rules_path = self.rules_file();
        match fs::read_to_string(&rules_path) {
            Ok(data) => match serde_json::from_str::<Vec<Rule>>(&data) {
                Ok(loaded) => {
                    if let Ok(mut lock) = self.rules_cache.write() {
                        *lock = loaded;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[proxy-guard] ERROR: Failed to parse rules file {:?}: {e}",
                        rules_path
                    );
                    self.rules_corrupted.store(true, Ordering::SeqCst);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "[proxy-guard] ERROR: Failed to read rules file {:?}: {e}",
                    rules_path
                );
                self.rules_corrupted.store(true, Ordering::SeqCst);
            }
        }

        let wl_path = self.whitelist_file();
        match fs::read_to_string(&wl_path) {
            Ok(data) => match serde_json::from_str::<Vec<String>>(&data) {
                Ok(loaded) => {
                    if let Ok(mut lock) = self.whitelist_cache.write() {
                        *lock = loaded;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[proxy-guard] ERROR: Failed to parse whitelist file {:?}: {e}",
                        wl_path
                    );
                    self.whitelist_corrupted.store(true, Ordering::SeqCst);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "[proxy-guard] ERROR: Failed to read whitelist file {:?}: {e}",
                    wl_path
                );
                self.whitelist_corrupted.store(true, Ordering::SeqCst);
            }
        }

        let metrics_path = self.metrics_file();
        match fs::read_to_string(&metrics_path) {
            Ok(data) => match serde_json::from_str::<FlowMetrics>(&data) {
                Ok(loaded) => {
                    if let Ok(mut lock) = self.metrics_cache.write() {
                        *lock = loaded;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[proxy-guard] ERROR: Failed to parse metrics file {:?}: {e}",
                        metrics_path
                    );
                    self.metrics_corrupted.store(true, Ordering::SeqCst);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "[proxy-guard] ERROR: Failed to read metrics file {:?}: {e}",
                    metrics_path
                );
                self.metrics_corrupted.store(true, Ordering::SeqCst);
            }
        }

        let proxies_path = self.proxies_file();
        match fs::read_to_string(&proxies_path) {
            Ok(data) => match serde_json::from_str::<Vec<UpstreamProxy>>(&data) {
                Ok(loaded) => {
                    if let Ok(mut lock) = self.proxies_cache.write() {
                        *lock = loaded;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[proxy-guard] ERROR: Failed to parse proxies file {:?}: {e}",
                        proxies_path
                    );
                    self.proxies_corrupted.store(true, Ordering::SeqCst);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "[proxy-guard] ERROR: Failed to read proxies file {:?}: {e}",
                    proxies_path
                );
                self.proxies_corrupted.store(true, Ordering::SeqCst);
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
        if self.rules_corrupted.load(Ordering::SeqCst) {
            return Err(format!(
                "Cannot overwrite unreadable/corrupted rules file {:?}; manual recovery required",
                self.rules_file()
            ));
        }
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
        if self.whitelist_corrupted.load(Ordering::SeqCst) {
            return Err(format!(
                "Cannot overwrite unreadable/corrupted whitelist file {:?}; manual recovery required",
                self.whitelist_file()
            ));
        }
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
        if self.metrics_corrupted.load(Ordering::SeqCst) {
            return Err(format!(
                "Cannot overwrite unreadable/corrupted metrics file {:?}; manual recovery required",
                self.metrics_file()
            ));
        }
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

impl ProxyPoolStoragePort for FileStorageAdapter {
    fn load_proxies(&self) -> Vec<UpstreamProxy> {
        self.proxies_cache
            .read()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    fn save_proxies(&self, proxies: &[UpstreamProxy]) -> Result<(), String> {
        if self.proxies_corrupted.load(Ordering::SeqCst) {
            return Err(format!(
                "Cannot overwrite unreadable/corrupted proxies file {:?}; manual recovery required",
                self.proxies_file()
            ));
        }
        let mut lock = self
            .proxies_cache
            .write()
            .map_err(|e| format!("Lock acquisition error: {e}"))?;
        let json = serde_json::to_string_pretty(proxies).map_err(|e| e.to_string())?;
        self.atomic_write_file(&self.proxies_file(), &json)?;
        *lock = proxies.to_vec();
        Ok(())
    }
}
