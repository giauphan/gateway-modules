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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub struct ProxyPool {
    storage: Arc<dyn ProxyPoolStoragePort>,
    proxies: RwLock<Vec<UpstreamProxy>>,
    strategy: RwLock<ProxyRotationStrategy>,
    rr_counter: AtomicUsize,
    sticky_sessions: RwLock<HashMap<String, String>>,
    mutation_lock: Mutex<()>,
}

impl ProxyPool {
    pub fn new(
        storage: Arc<dyn ProxyPoolStoragePort>,
        initial_strategy: ProxyRotationStrategy,
    ) -> Self {
        let loaded = storage.load_proxies();
        Self {
            storage,
            proxies: RwLock::new(loaded),
            strategy: RwLock::new(initial_strategy),
            rr_counter: AtomicUsize::new(0),
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

    fn filter_candidates(
        all_proxies: Vec<UpstreamProxy>,
        query: &ProxySelectionQuery,
    ) -> Vec<UpstreamProxy> {
        let mut candidates: Vec<UpstreamProxy> =
            all_proxies.into_iter().filter(|p| p.enabled).collect();

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
                    if let Ok(sessions) = self.sticky_sessions.read() {
                        if let Some(assigned_id) = sessions.get(session_id) {
                            if let Some(p) = candidates.iter().find(|p| p.id == *assigned_id) {
                                return Some(p.clone());
                            }
                        }
                    }
                    let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed) % candidates.len();
                    let selected = candidates[idx].clone();
                    if let Ok(mut sessions) = self.sticky_sessions.write() {
                        sessions.insert(session_id.clone(), selected.id.clone());
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
                let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed) % candidates.len();
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
        self.storage.save_proxies(&proxies)?;
        if let Ok(mut write_lock) = self.proxies.write() {
            *write_lock = proxies;
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
            sessions.retain(|_, v| v != id);
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
                return Some(found.clone());
            }
        }

        let candidates = Self::filter_candidates(all_proxies, query);
        if candidates.is_empty() {
            return None;
        }

        let strategy = self
            .strategy
            .read()
            .map(|s| *s)
            .unwrap_or(ProxyRotationStrategy::RoundRobin);

        self.apply_strategy(strategy, &candidates, query.session_id.as_ref())
    }

    fn select_fallback(
        &self,
        failed_id: &str,
        query: &ProxySelectionQuery,
    ) -> Option<UpstreamProxy> {
        let all_proxies = self.list_proxies();
        let fallback_candidates: Vec<UpstreamProxy> = all_proxies
            .into_iter()
            .filter(|p| p.enabled && p.id != failed_id)
            .collect();

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
        }

        // Select the one with the lowest failures
        fallback_candidates
            .into_iter()
            .min_by_key(|p| p.failure_count)
    }

    fn record_proxy_result(&self, id: &str, success: bool) {
        if let Ok(mut list) = self.proxies.write() {
            if let Some(p) = list.iter_mut().find(|p| p.id == id) {
                if success {
                    p.success_count = p.success_count.saturating_add(1);
                    if p.failure_count > 0 {
                        p.failure_count = p.failure_count.saturating_sub(1);
                    }
                } else {
                    p.failure_count = p.failure_count.saturating_add(1);
                }
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
