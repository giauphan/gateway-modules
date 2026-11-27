//! Multi-Proxy Pool Manager and Rotation Engine.
//!
//! Clean Architecture: Application Layer Orchestrator.
//! Manages multiple upstream proxies (residential, datacenter, mobile), health status,
//! rotation strategies (Round-Robin, Sticky Session, Lowest Failures), and automatic failover.

use crate::domain::{
    PlaywrightProxyConfig, ProxyRotationStrategy, ProxySelectionQuery, UpstreamProxy,
};
use crate::ports::{ProxyPoolManagementPort, ProxyPoolStoragePort};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const MAX_STICKY_SESSIONS: usize = 10_000;
const DEFAULT_STICKY_TTL_SECS: u64 = 600;

#[derive(Debug, Clone)]
struct StickyAssignment {
    proxy_id: String,
    session_token: String,
    expires_at: Instant,
}

pub struct ProxyPool {
    storage: Arc<dyn ProxyPoolStoragePort>,
    proxies: RwLock<Vec<UpstreamProxy>>,
    strategy: RwLock<ProxyRotationStrategy>,
    rr_counter: AtomicUsize,
    session_epoch: AtomicU64,
    sticky_sessions: RwLock<HashMap<String, StickyAssignment>>,
    mutation_lock: Mutex<()>,
}

impl ProxyPool {
    pub fn new(
        storage: Arc<dyn ProxyPoolStoragePort>,
        initial_strategy: ProxyRotationStrategy,
    ) -> Self {
        let loaded = storage.load_proxies();
        let initial_rr = storage.load_rr_counter();
        Self {
            storage,
            proxies: RwLock::new(loaded),
            strategy: RwLock::new(initial_strategy),
            rr_counter: AtomicUsize::new(initial_rr),
            session_epoch: AtomicU64::new(1),
            sticky_sessions: RwLock::new(HashMap::new()),
            mutation_lock: Mutex::new(()),
        }
    }

    /// Convenience helper to retrieve next proxy formatted directly for Playwright/Puppeteer tools.
    pub fn get_next_playwright_config(
        &self,
        query: &ProxySelectionQuery,
    ) -> Option<PlaywrightProxyConfig> {
        self.select_proxy(query).map(|p| p.to_playwright_config())
    }

    /// Resolves the active sticky session token for a selected proxy, honoring TTL expiry
    /// and auto-generating a pinned internal session ID when `sticky_session.enabled == true`.
    fn resolve_effective_session_token(
        &self,
        selected: &UpstreamProxy,
        query: &ProxySelectionQuery,
    ) -> Option<String> {
        let now = Instant::now();
        let sticky_cfg = selected.sticky_session.as_ref().filter(|c| c.enabled);

        if let Some(ref sid) = query.session_id {
            let trimmed = sid.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Some(cfg) = sticky_cfg {
                let key = format!(
                    "sid:{}:{}",
                    selected.pool_name.as_deref().unwrap_or("default"),
                    trimmed
                );
                if let Ok(sessions) = self.sticky_sessions.read() {
                    if let Some(existing) = sessions.get(&key) {
                        if now < existing.expires_at {
                            return Some(existing.session_token.clone());
                        }
                    }
                }
                let epoch = self.session_epoch.fetch_add(1, Ordering::Relaxed);
                let token = format!("{trimmed}-r{epoch}");
                let ttl = Duration::from_secs(cfg.ttl_secs.max(1));
                if let Ok(mut sessions) = self.sticky_sessions.write() {
                    if sessions.len() >= MAX_STICKY_SESSIONS {
                        sessions.retain(|_, v| now < v.expires_at);
                    }
                    sessions.insert(
                        key,
                        StickyAssignment {
                            proxy_id: selected.id.clone(),
                            session_token: token.clone(),
                            expires_at: now + ttl,
                        },
                    );
                }
                return Some(token);
            }
            return Some(trimmed.to_string());
        }

        // No client session_id supplied: auto-generate pinned session when sticky_session is enabled
        if let Some(cfg) = sticky_cfg {
            let auto_key = format!(
                "auto:{}",
                selected.pool_name.as_deref().unwrap_or(&selected.id)
            );
            if let Ok(sessions) = self.sticky_sessions.read() {
                if let Some(existing) = sessions.get(&auto_key) {
                    if now < existing.expires_at {
                        return Some(existing.session_token.clone());
                    }
                }
            }
            let epoch = self.session_epoch.fetch_add(1, Ordering::Relaxed);
            let token = format!("auto-session-{epoch}");
            let ttl = Duration::from_secs(cfg.ttl_secs.max(1));
            if let Ok(mut sessions) = self.sticky_sessions.write() {
                if sessions.len() >= MAX_STICKY_SESSIONS {
                    sessions.retain(|_, v| now < v.expires_at);
                }
                sessions.insert(
                    auto_key,
                    StickyAssignment {
                        proxy_id: selected.id.clone(),
                        session_token: token.clone(),
                        expires_at: now + ttl,
                    },
                );
            }
            return Some(token);
        }

        None
    }

    fn filter_candidates(
        all_proxies: Vec<UpstreamProxy>,
        query: &ProxySelectionQuery,
    ) -> Vec<UpstreamProxy> {
        let mut candidates: Vec<UpstreamProxy> =
            all_proxies.into_iter().filter(|p| p.enabled).collect();

        // Explicit named pool filter (strict / fail-closed so sensitive traffic never leaks to wrong pool)
        if let Some(ref pool) = query.pool {
            let trimmed = pool.trim();
            if !trimmed.is_empty() {
                candidates.retain(|p| p.matches_pool(trimmed));
                if candidates.is_empty() {
                    return Vec::new();
                }
            }
        }

        if let Some(ref c) = query.country {
            let target_country = c.trim().to_ascii_uppercase();
            let country_matches: Vec<UpstreamProxy> = candidates
                .iter()
                .filter(|p| {
                    p.country
                        .as_ref()
                        .map(|pc| pc.eq_ignore_ascii_case(&target_country))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if !country_matches.is_empty() {
                candidates = country_matches;
            } else {
                // If no exact match exists, adapt dynamic residential proxies (e.g. Oxylabs)
                let adapted: Vec<UpstreamProxy> = candidates
                    .iter()
                    .filter_map(|p| p.adapt_country(&target_country))
                    .collect();
                if !adapted.is_empty() {
                    candidates = adapted;
                } else {
                    // Fail closed if the requested country cannot be satisfied
                    return Vec::new();
                }
            }
        }

        if let Some(ref tag) = query.tag {
            let tag_matches: Vec<UpstreamProxy> = candidates
                .iter()
                .filter(|p| p.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
                .cloned()
                .collect();
            if !tag_matches.is_empty() {
                candidates = tag_matches;
            }
        }

        candidates
    }

    fn apply_strategy(
        &self,
        strategy: ProxyRotationStrategy,
        candidates: &[UpstreamProxy],
        session_id: Option<&String>,
    ) -> Option<UpstreamProxy> {
        if candidates.is_empty() {
            return None;
        }
        match strategy {
            ProxyRotationStrategy::StickySession => {
                if let Some(session_id) = session_id {
                    let now = Instant::now();
                    if let Ok(sessions) = self.sticky_sessions.read() {
                        if let Some(assignment) = sessions.get(session_id) {
                            if now < assignment.expires_at {
                                if let Some(p) =
                                    candidates.iter().find(|p| p.id == assignment.proxy_id)
                                {
                                    return Some(p.clone());
                                }
                            }
                        }
                    }
                    let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed) % candidates.len();
                    let selected = candidates[idx].clone();
                    let ttl_secs = selected
                        .sticky_session
                        .as_ref()
                        .map(|s| s.ttl_secs)
                        .unwrap_or(DEFAULT_STICKY_TTL_SECS);
                    let epoch = self.session_epoch.fetch_add(1, Ordering::Relaxed);
                    let token = if epoch == 1 {
                        session_id.clone()
                    } else {
                        format!("{session_id}:{epoch}")
                    };
                    if let Ok(mut sessions) = self.sticky_sessions.write() {
                        if sessions.len() >= MAX_STICKY_SESSIONS {
                            sessions.retain(|_, v| now < v.expires_at);
                            if sessions.len() >= MAX_STICKY_SESSIONS {
                                sessions.clear();
                            }
                        }
                        sessions.insert(
                            session_id.clone(),
                            StickyAssignment {
                                proxy_id: selected.id.clone(),
                                session_token: token,
                                expires_at: now + Duration::from_secs(ttl_secs),
                            },
                        );
                    }
                    Some(selected)
                } else {
                    let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed) % candidates.len();
                    Some(candidates[idx].clone())
                }
            }
            ProxyRotationStrategy::LowestFailures => {
                candidates.iter().min_by_key(|p| p.failure_count).cloned()
            }
            ProxyRotationStrategy::Random => {
                if candidates.len() <= 1 {
                    return candidates.first().cloned();
                }
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as usize)
                    .unwrap_or(0);
                let count = self.rr_counter.fetch_add(1, Ordering::Relaxed);
                let mixed = (nanos.rotate_left(17) ^ count).wrapping_mul(0x9e3779b97f4a7c15);
                let idx = mixed % candidates.len();
                Some(candidates[idx].clone())
            }
            ProxyRotationStrategy::RoundRobin => {
                let current = self.rr_counter.fetch_add(1, Ordering::Relaxed);
                let next = current.wrapping_add(1);
                if let Err(e) = self.storage.save_rr_counter(next) {
                    eprintln!("[proxy-guard] Warning: Failed to persist round-robin counter: {e}");
                }
                let idx = current % candidates.len();
                Some(candidates[idx].clone())
            }
        }
    }
}

impl ProxyPoolManagementPort for ProxyPool {
    fn add_proxy(&self, proxy: UpstreamProxy) -> Result<String, String> {
        let _guard = self.mutation_lock.lock().map_err(|e| e.to_string())?;
        let proxy_id = proxy.id.clone();

        let mut list = self.proxies.read().map(|p| p.clone()).unwrap_or_default();

        // If proxy with same ID already exists, replace it; otherwise append
        if let Some(pos) = list.iter().position(|p| p.id == proxy_id) {
            list[pos] = proxy;
        } else {
            list.push(proxy);
        }

        self.storage.save_proxies(&list)?;

        if let Ok(mut write_lock) = self.proxies.write() {
            *write_lock = list;
        }

        Ok(proxy_id)
    }

    fn replace_proxies(&self, proxies: Vec<UpstreamProxy>) -> Result<(), String> {
        let _guard = self.mutation_lock.lock().map_err(|e| e.to_string())?;
        let mut unique_proxies: Vec<UpstreamProxy> = Vec::with_capacity(proxies.len());
        let mut seen = std::collections::HashSet::with_capacity(proxies.len());
        for p in proxies {
            if seen.insert(p.id.clone()) {
                unique_proxies.push(p);
            }
        }
        self.storage.save_proxies(&unique_proxies)?;
        if let Ok(mut write_lock) = self.proxies.write() {
            *write_lock = unique_proxies;
        }
        Ok(())
    }

    fn remove_proxy(&self, id: &str) -> Result<(), String> {
        let _guard = self.mutation_lock.lock().map_err(|e| e.to_string())?;

        let mut list = self.proxies.read().map(|p| p.clone()).unwrap_or_default();

        let initial_len = list.len();
        list.retain(|p| p.id != id);

        if list.len() == initial_len {
            return Err(format!("Proxy with ID '{id}' not found in pool"));
        }

        self.storage.save_proxies(&list)?;

        if let Ok(mut write_lock) = self.proxies.write() {
            *write_lock = list;
        }

        // Clean up sticky session references
        if let Ok(mut sessions) = self.sticky_sessions.write() {
            sessions.retain(|_, v| v.proxy_id != id);
        }

        Ok(())
    }

    fn list_proxies(&self) -> Vec<UpstreamProxy> {
        self.proxies.read().map(|p| p.clone()).unwrap_or_default()
    }

    fn select_proxy(&self, query: &ProxySelectionQuery) -> Option<UpstreamProxy> {
        let all_proxies = self.list_proxies();
        if all_proxies.is_empty() {
            return None;
        }

        // 1. Direct ID match if requested
        if let Some(ref pref_id) = query.preferred_id {
            if let Some(found) = all_proxies.iter().find(|p| p.id == *pref_id && p.enabled) {
                let selected = if let Some(ref sid) = query.session_id {
                    found.with_sticky_session(sid)
                } else {
                    found.clone()
                };
                return Some(selected);
            }
        }

        let candidates = Self::filter_candidates(all_proxies, query);
        if candidates.is_empty() {
            return None;
        }

        let strategy = query.strategy.unwrap_or_else(|| {
            self.strategy
                .read()
                .map(|s| *s)
                .unwrap_or(ProxyRotationStrategy::RoundRobin)
        });

        let selected = self.apply_strategy(strategy, &candidates, query.session_id.as_ref())?;
        let effective_session = self.resolve_effective_session_token(&selected, query);
        if let Some(token) = effective_session {
            Some(selected.with_sticky_session_for(&token, query.country.as_deref()))
        } else {
            Some(selected)
        }
    }

    fn select_fallback(
        &self,
        failed_id: &str,
        query: &ProxySelectionQuery,
    ) -> Option<UpstreamProxy> {
        let all_proxies = self.list_proxies();
        let mut fallback_candidates: Vec<UpstreamProxy> = all_proxies
            .into_iter()
            .filter(|p| p.enabled && p.id != failed_id)
            .collect();

        if let Some(ref pool) = query.pool {
            let trimmed = pool.trim();
            if !trimmed.is_empty() {
                fallback_candidates.retain(|p| p.matches_pool(trimmed));
            }
        }

        if fallback_candidates.is_empty() {
            return None;
        }

        // Apply country constraint if possible
        if let Some(ref c) = query.country {
            let target_country = c.trim().to_ascii_uppercase();
            if let Some(p) = fallback_candidates
                .iter()
                .find(|p| p.country.as_deref() == Some(&target_country))
            {
                return Some(p.clone());
            }
            if let Some(p) = fallback_candidates
                .iter()
                .find_map(|p| p.adapt_country(&target_country))
            {
                return Some(p);
            }
        }

        // Select the one with the lowest failures
        fallback_candidates
            .into_iter()
            .min_by_key(|p| p.failure_count)
    }

    fn record_proxy_result(&self, id: &str, success: bool) {
        let updated_snapshot = if let Ok(mut list) = self.proxies.write() {
            let target_pos = list.iter().position(|p| p.id == id).or_else(|| {
                // Fallback for dynamically country-adapted residential proxy IDs (e.g. `host:port:user-cc-vn`)
                let mut parts = id.splitn(3, ':');
                let host = parts.next()?;
                let port = parts.next()?.parse::<u16>().ok()?;
                list.iter().position(|p| p.host == host && p.port == port)
            });

            if let Some(pos) = target_pos {
                let p = &mut list[pos];
                if success {
                    p.success_count = p.success_count.saturating_add(1);
                    if p.failure_count > 0 {
                        p.failure_count = p.failure_count.saturating_sub(1);
                    }
                } else {
                    p.failure_count = p.failure_count.saturating_add(1);
                }
                Some(list.clone())
            } else {
                None
            }
        } else {
            None
        };

        if let Some(snapshot) = updated_snapshot {
            if let Err(e) = self.storage.save_proxies(&snapshot) {
                eprintln!("[proxy-guard] Warning: Failed to persist proxy health result: {e}");
            }
        }
    }

    fn set_strategy(&self, strategy: ProxyRotationStrategy) -> Result<(), String> {
        let mut lock = self
            .strategy
            .write()
            .map_err(|e| format!("Strategy lock acquisition error: {e}"))?;
        *lock = strategy;
        Ok(())
    }

    fn get_strategy(&self) -> ProxyRotationStrategy {
        self.strategy
            .read()
            .map(|s| *s)
            .unwrap_or(ProxyRotationStrategy::RoundRobin)
    }
}
