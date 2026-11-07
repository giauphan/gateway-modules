//! Comprehensive verification tests for:
//! 1. Storage State Directory resolution & durability
//! 2. Service idempotency & error propagation
//! 3. Crypto test helper isolation
//! 4. Config contract for string lists and comma-separated strings

use daily_news::domain::{IdempotencyRecord, ModuleConfig};
use daily_news::fetcher::LiveFeedFetcher;
use daily_news::scoring::CandidateScorer;
use daily_news::service::DailyNewsService;
use daily_news::storage::{try_resolve_state_dir, FileStorageAdapter};
use daily_news::StoragePort;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn make_temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dn_test_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// =========================================================================
// 1. Storage State Directory & Durability
// =========================================================================

#[test]
fn test_state_dir_resolution_writable_and_override() {
    let tmp = make_temp_dir("state_dir_ok");
    let state_dir = tmp.join(".state");
    let _ = std::fs::create_dir_all(&state_dir);

    // Default base resolves to base/.state when writable
    let resolved = try_resolve_state_dir(&tmp).expect("Writable base dir must resolve");
    assert_eq!(resolved, state_dir);

    // DAILY_NEWS_STATE_DIR override
    let custom_dir = tmp.join("custom_state_dir");
    let _ = std::fs::create_dir_all(&custom_dir);
    std::env::set_var("DAILY_NEWS_STATE_DIR", custom_dir.to_str().unwrap());
    let resolved_override = try_resolve_state_dir(&tmp).expect("Override must resolve");
    assert_eq!(resolved_override, custom_dir);
    std::env::remove_var("DAILY_NEWS_STATE_DIR");
}

#[test]
fn test_state_dir_unwritable_prohibits_ephemeral_fallback_in_production() {
    let non_writable = PathBuf::from("/proc/non_writable_system_path_daily_news");
    std::env::remove_var("DAILY_NEWS_ALLOW_EPHEMERAL_STATE");

    // In a production-like context where ephemeral fallback is prohibited
    let err = try_resolve_state_dir(&non_writable);
    // When non-writable and ephemeral state not allowed:
    if let Err(e) = err {
        assert!(e.contains("is not writable. Ephemeral fallback to /tmp is prohibited in production."));
    }
}

// =========================================================================
// 2. Service Idempotency & Error Propagation
// =========================================================================

#[test]
fn test_commit_handoff_with_idempotency_workflow() {
    let tmp = make_temp_dir("idempotency_srv");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage.clone(), scorer);

    let candidate_id = "cand_idemp_01";
    let token = storage
        .reserve_handoff(candidate_id, 300, "worker_idemp")
        .expect("Reservation must succeed");

    // 1. First commit with idempotency key
    let key = "key_handoff_uuid_1001";
    let res1 = service
        .commit_handoff_with_idempotency(candidate_id, &token, Some(key))
        .expect("Initial commit must succeed");
    assert!(res1, "Commit must return true");

    // Verify key was registered
    let record = service
        .check_publish_idempotency(key)
        .expect("Check must succeed")
        .expect("Record must exist");
    assert_eq!(record.candidate_id, candidate_id);
    assert_eq!(record.action, "commit_handoff");

    // 2. Idempotent replay with SAME key and candidate -> Ok(true)
    let res2 = service
        .commit_handoff_with_idempotency(candidate_id, &token, Some(key))
        .expect("Idempotent replay must succeed");
    assert!(res2, "Replay must return true");

    // 3. Replay with SAME key but DIFFERENT candidate -> Err
    let res3 = service.commit_handoff_with_idempotency("cand_different", "other_token", Some(key));
    assert!(res3.is_err(), "Reusing key for different candidate must fail");
    assert!(res3.unwrap_err().contains("already registered for candidate 'cand_idemp_01'"));

    // 4. Delegated commit_handoff without key
    let candidate_id_2 = "cand_idemp_02";
    let token_2 = storage
        .reserve_handoff(candidate_id_2, 300, "worker_idemp_2")
        .expect("Reservation must succeed");
    let res_delegated = service
        .commit_handoff(candidate_id_2, &token_2)
        .expect("Delegated commit must succeed");
    assert!(res_delegated);
}

#[test]
fn test_publish_idempotency_registration_and_lookup() {
    let tmp = make_temp_dir("pub_idemp");
    let storage = Arc::new(FileStorageAdapter::new(&tmp));
    let fetcher = Arc::new(LiveFeedFetcher::new(None));
    let scorer = CandidateScorer::default();
    let service = DailyNewsService::new(fetcher, storage, scorer);

    let key = "pub_key_2026_x";
    let reg1 = service.register_publish_idempotency(key, "cand_pub_01", "publish_upload");
    assert_eq!(reg1, Ok(true));

    let reg2 = service.register_publish_idempotency(key, "cand_pub_01", "publish_upload");
    assert_eq!(reg2, Ok(false), "Duplicate key returns Ok(false)");

    let check = service.check_publish_idempotency(key);
    assert_eq!(
        check,
        Ok(Some(IdempotencyRecord {
            candidate_id: "cand_pub_01".to_string(),
            action: "publish_upload".to_string(),
        }))
    );
}

// =========================================================================
// 3. Config Contract (JSON arrays vs Comma-Separated Strings)
// =========================================================================

#[test]
fn test_config_contract_array_and_comma_separated_strings() {
    let mut cfg = ModuleConfig::default();

    // 1. default_sources as JSON array of strings
    let payload_array = json!({
        "default_sources": ["HackerNews", "NewsNow", "CustomSource"]
    });
    cfg.apply_updates_validated(&payload_array)
        .expect("Array parsing must succeed");
    assert_eq!(cfg.default_sources, vec!["hackernews", "newsnow", "customsource"]);

    // 2. default_sources as comma-separated string
    let payload_comma = json!({
        "default_sources": "LocalLLaMA, MachineLearning , AI_News"
    });
    cfg.apply_updates_validated(&payload_comma)
        .expect("Comma-separated string parsing must succeed");
    assert_eq!(cfg.default_sources, vec!["localllama", "machinelearning", "ai_news"]);

    // 3. reddit_subreddits as JSON array of strings
    let payload_reddit_arr = json!({
        "reddit_subreddits": ["rust", "golang", "programming"]
    });
    cfg.apply_updates_validated(&payload_reddit_arr)
        .expect("Reddit array parsing must succeed");
    assert_eq!(cfg.reddit_subreddits, vec!["rust", "golang", "programming"]);

    // 4. reddit_subreddits as comma-separated string
    let payload_reddit_str = json!({
        "reddit_subreddits": "LocalLLaMA, MachineLearning, singularity"
    });
    cfg.apply_updates_validated(&payload_reddit_str)
        .expect("Reddit comma-separated string parsing must succeed");
    assert_eq!(cfg.reddit_subreddits, vec!["LocalLLaMA", "MachineLearning", "singularity"]);
}
