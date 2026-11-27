//! File and In-Memory Storage Adapter.
//!
//! Clean Architecture: Interface Adapters / Infrastructure Layer.
//! Implements RuleRepositoryPort and MetricsStoragePort with atomic file replacement.

use crate::domain::{DefaultRoute, FlowMetrics, Rule, UpstreamProxy};
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
    default_route_cache: RwLock<DefaultRoute>,
    metrics_cache: RwLock<FlowMetrics>,
    proxies_cache: RwLock<Vec<UpstreamProxy>>,
    rules_corrupted: AtomicBool,
    whitelist_corrupted: AtomicBool,
    default_route_corrupted: AtomicBool,
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
            default_route_cache: RwLock::new(DefaultRoute::ForwardUpstream),
            metrics_cache: RwLock::new(FlowMetrics::new(3.5)),
            proxies_cache: RwLock::new(Vec::new()),
            rules_corrupted: AtomicBool::new(false),
            whitelist_corrupted: AtomicBool::new(false),
            default_route_corrupted: AtomicBool::new(false),
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

    fn default_route_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("default_route.json")
    }

    fn metrics_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("metrics.json")
    }

    fn proxies_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("proxies.json")
    }

    fn rr_counter_file(&self) -> PathBuf {
        self.base_dir.join(".state").join("rr_counter.txt")
    }

    pub fn config_file(&self) -> PathBuf {
        if let Ok(env_path) = std::env::var("PROXY_GUARD_CONFIG_PATH") {
            let p = PathBuf::from(env_path);
            if p.is_file() {
                return p;
            }
        }
        let local_cfg = self.base_dir.join("config.json");
        if local_cfg.is_file() {
            return local_cfg;
        }
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_cfg = cwd.join("config.json");
            if cwd_cfg.is_file() {
                return cwd_cfg;
            }
        }
        self.base_dir.join("config.json")
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

        let dr_path = self.default_route_file();
        match fs::read_to_string(&dr_path) {
            Ok(data) => match serde_json::from_str::<DefaultRoute>(&data) {
                Ok(loaded) => {
                    if let Ok(mut lock) = self.default_route_cache.write() {
                        *lock = loaded;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[proxy-guard] ERROR: Failed to parse default_route file {:?}: {e}",
                        dr_path
                    );
                    self.default_route_corrupted.store(true, Ordering::SeqCst);
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!(
                    "[proxy-guard] ERROR: Failed to read default_route file {:?}: {e}",
                    dr_path
                );
                self.default_route_corrupted.store(true, Ordering::SeqCst);
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
                Ok(mut loaded) => {
                    for p in &mut loaded {
                        if let Some(ref a) = p.auth {
                            let expected_id = format!("{}:{}:{}", p.host, p.port, a.username);
                            if p.id == format!("{}:{}", p.host, p.port) {
                                p.id = expected_id;
                            }
                        }
                    }
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

        let cfg_path = self.config_file();
        if cfg_path.is_file() {
            if let Ok(data) = fs::read_to_string(&cfg_path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                    self.sync_from_config_json(&val);
                }
            }
        }
    }

    fn sync_from_config_json(&self, val: &serde_json::Value) {
        let mut proxies_to_add = Vec::new();

        // 1. upstream_proxy_pool
        if let Some(pool_raw) = val.get("upstream_proxy_pool").and_then(|v| v.as_str()) {
            for line in pool_raw.split(['\n', ',', ';']) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    if let Ok(proxy) = UpstreamProxy::parse(trimmed, None, None, None) {
                        proxies_to_add.push(proxy);
                    }
                }
            }
        }

        // 2. upstream_proxy_url (fallback)
        if let Some(single_url) = val.get("upstream_proxy_url").and_then(|v| v.as_str()) {
            let trimmed = single_url.trim();
            if !trimmed.is_empty() {
                if let Ok(proxy) = UpstreamProxy::parse(trimmed, None, None, None) {
                    proxies_to_add.push(proxy);
                }
            }
        }

        // Add to proxies_cache if not already present
        if !proxies_to_add.is_empty() {
            if let Ok(mut lock) = self.proxies_cache.write() {
                let mut changed = false;
                for p in &mut *lock {
                    if let Some(ref a) = p.auth {
                        let expected_id = format!("{}:{}:{}", p.host, p.port, a.username);
                        if p.id == format!("{}:{}", p.host, p.port) {
                            p.id = expected_id;
                            changed = true;
                        }
                    }
                }
                for p in proxies_to_add {
                    if !lock
                        .iter()
                        .any(|existing| existing.id == p.id || existing.url == p.url)
                    {
                        lock.push(p);
                        changed = true;
                    }
                }
                if changed && !self.proxies_corrupted.load(Ordering::SeqCst) {
                    if let Ok(json) = serde_json::to_string_pretty(&*lock) {
                        let _ = self.atomic_write_file(&self.proxies_file(), &json);
                    }
                }
            }
        }

        // 3. cost_per_gb_usd
        if let Some(cost) = val.get("cost_per_gb_usd").and_then(|v| v.as_f64()) {
            if cost > 0.0 {
                if let Ok(mut lock) = self.metrics_cache.write() {
                    lock.cost_per_gb_usd = cost;
                }
            }
        }

        // 4. whitelist_domains
        if let Some(arr) = val.get("whitelist_domains").and_then(|v| v.as_array()) {
            if let Ok(mut lock) = self.whitelist_cache.write() {
                for item in arr {
                    if let Some(d) = item.as_str() {
                        let trimmed = d.trim();
                        if !trimmed.is_empty() && !lock.contains(&trimmed.to_string()) {
                            lock.push(trimmed.to_string());
                        }
                    }
                }
            }
        }

        // 5. Rule category toggles
        let toggles = [
            ("block_analytics", crate::domain::RuleCategory::Analytics),
            ("block_ads", crate::domain::RuleCategory::Advertising),
            ("block_telemetry", crate::domain::RuleCategory::Telemetry),
            ("block_media", crate::domain::RuleCategory::HeavyMedia),
            ("block_fonts", crate::domain::RuleCategory::Font),
        ];
        if let Ok(mut lock) = self.rules_cache.write() {
            for (key, cat) in toggles {
                if let Some(enabled) = val.get(key).and_then(|v| v.as_bool()) {
                    for r in lock.iter_mut() {
                        if r.category == cat {
                            r.enabled = enabled;
                        }
                    }
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

    fn load_default_route(&self) -> DefaultRoute {
        self.default_route_cache
            .read()
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    fn save_default_route(&self, route: &DefaultRoute) -> Result<(), String> {
        if self.default_route_corrupted.load(Ordering::SeqCst) {
            return Err(format!(
                "Cannot overwrite unreadable/corrupted default_route file {:?}; manual recovery required",
                self.default_route_file()
            ));
        }
        let mut lock = self
            .default_route_cache
            .write()
            .map_err(|e| format!("Lock acquisition error: {e}"))?;
        let json = serde_json::to_string_pretty(route).map_err(|e| e.to_string())?;
        self.atomic_write_file(&self.default_route_file(), &json)?;
        *lock = route.clone();
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

    fn load_rr_counter(&self) -> usize {
        fs::read_to_string(self.rr_counter_file())
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0)
    }

    fn save_rr_counter(&self, counter: usize) -> Result<(), String> {
        self.atomic_write_file(&self.rr_counter_file(), &counter.to_string())
    }
}
