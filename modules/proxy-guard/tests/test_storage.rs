//! Tests for FileStorageAdapter persistence, whitelist, rules, metrics, and proxy state.

use proxy_guard::domain::{
    FlowMetrics, PatternType, ProxyProtocol, Rule, RuleAction, RuleCategory, UpstreamProxy,
};
use proxy_guard::ports::{MetricsStoragePort, ProxyPoolStoragePort, RuleRepositoryPort};
use proxy_guard::storage::FileStorageAdapter;
use std::fs;
use std::path::PathBuf;

struct TempStorageDir {
    path: PathBuf,
}

impl TempStorageDir {
    fn new(suffix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pg_storage_test_{}_{}_{}",
            suffix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::create_dir_all(&path);
        Self { path }
    }
}

impl Drop for TempStorageDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_storage_rules_and_whitelist() {
    let temp = TempStorageDir::new("rules");
    let storage = FileStorageAdapter::new(&temp.path);

    // Initial builtin rules loaded
    let rules = storage.load_rules();
    assert!(!rules.is_empty(), "Builtin rules must be loaded");

    // Add a custom rule and save
    let mut modified_rules = rules.clone();
    let custom = Rule::new(
        "custom-1",
        "Custom Block",
        RuleCategory::Custom,
        PatternType::DomainSuffix,
        "custom-tracker.com",
        RuleAction::Block403,
        100,
        "Custom rule description",
    );
    modified_rules.push(custom);
    assert!(storage.save_rules(&modified_rules).is_ok());

    let updated_rules = storage.load_rules();
    assert!(updated_rules.iter().any(|r| r.id == "custom-1"));

    // Remove custom rule and save
    modified_rules.retain(|r| r.id != "custom-1");
    assert!(storage.save_rules(&modified_rules).is_ok());
    let after_remove = storage.load_rules();
    assert!(!after_remove.iter().any(|r| r.id == "custom-1"));

    // Whitelist management
    let initial_wl = storage.load_whitelist();
    assert!(!initial_wl.contains(&"www.google-analytics.com".to_string()));

    let updated_wl = vec!["www.google-analytics.com".to_string()];
    assert!(storage.save_whitelist(&updated_wl).is_ok());
    assert_eq!(storage.load_whitelist(), updated_wl);

    // Reload from disk to verify persistence roundtrip
    let reloaded = FileStorageAdapter::new(&temp.path);
    assert_eq!(reloaded.load_whitelist(), updated_wl);

    // Remove from whitelist
    assert!(storage.save_whitelist(&[]).is_ok());
    assert!(storage.load_whitelist().is_empty());
}

#[test]
fn test_storage_metrics_persistence() {
    let temp = TempStorageDir::new("metrics");
    let storage = FileStorageAdapter::new(&temp.path);

    let m1 = FlowMetrics {
        total_requests: 100,
        blocked_requests: 70,
        forwarded_requests: 30,
        direct_bypassed_requests: 0,
        bytes_saved: 5_000_000,
        upstream_bytes_used: 100_000,
        cost_per_gb_usd: 3.5,
        estimated_cost_saved_usd: 0.0175,
        ..Default::default()
    };

    assert!(storage.save_metrics(&m1).is_ok());
    let loaded = storage.load_metrics();
    assert_eq!(loaded.total_requests, 100);
    assert_eq!(loaded.blocked_requests, 70);
    assert_eq!(loaded.bytes_saved, 5_000_000);

    // Reset metrics by saving default
    assert!(storage.save_metrics(&FlowMetrics::default()).is_ok());
    let reset = storage.load_metrics();
    assert_eq!(reset.total_requests, 0);
    assert_eq!(reset.bytes_saved, 0);
}

#[test]
fn test_storage_proxies_persistence_and_permissions() {
    let temp = TempStorageDir::new("proxies");
    let storage = FileStorageAdapter::new(&temp.path);

    let p1 = UpstreamProxy::parse(
        "http://alice:secret@proxy1.com:8080",
        Some("p1"),
        Some("US"),
        Some(vec!["fast".to_string()]),
    )
    .expect("Valid proxy");

    let p2 = UpstreamProxy::parse("socks5://proxy2.com:1080", Some("p2"), Some("DE"), None)
        .expect("Valid socks5 proxy");

    assert!(storage.save_proxies(&[p1, p2]).is_ok());

    // Verify file permissions on Unix are 0o600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let proxy_file = temp.path.join(".state").join("proxies.json");
        let metadata = fs::metadata(&proxy_file).expect("File must exist");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "State file permissions must be 0600 (owner-only)"
        );
    }

    // Verify reloading reads saved proxies
    let reloaded = FileStorageAdapter::new(&temp.path);
    let loaded_proxies = reloaded.load_proxies();
    assert_eq!(loaded_proxies.len(), 2);
    assert_eq!(loaded_proxies[0].host, "proxy1.com");
    assert_eq!(loaded_proxies[1].protocol, ProxyProtocol::Socks5);
}

#[test]
fn test_storage_corrupted_file_recovery() {
    let temp = TempStorageDir::new("corrupted");
    let state_dir = temp.path.join(".state");
    fs::create_dir_all(&state_dir).expect("Directory created");

    // Write completely corrupted JSON into state files
    fs::write(state_dir.join("rules.json"), b"INVALID JSON {[[{").expect("Corrupted rules written");
    fs::write(state_dir.join("whitelist.json"), b"INVALID JSON")
        .expect("Corrupted whitelist written");
    fs::write(state_dir.join("metrics.json"), b"{ not json }").expect("Corrupted metrics written");
    fs::write(state_dir.join("proxies.json"), b"broken").expect("Corrupted proxies written");

    // Loading corrupted files must not panic and must fall back gracefully
    let storage = FileStorageAdapter::new(&temp.path);
    let rules = storage.load_rules();
    assert!(!rules.is_empty(), "Built-in fallback rules must be loaded");
    let metrics = storage.load_metrics();
    assert_eq!(metrics.total_requests, 0);
    let proxies = storage.load_proxies();
    assert!(proxies.is_empty());
}
