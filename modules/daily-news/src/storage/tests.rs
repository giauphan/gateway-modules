use super::*;
use crate::domain::NormalizedCandidate;
use crate::ports::StoragePort;
use std::fs;

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
        assert_eq!(recovered.len(), 10, "All 10 candidates must survive restart");
        let cand_3 = restarted_storage
            .get_candidate("cand_3")
            .expect("cand_3 must survive");
        assert_eq!(
            cand_3.body.as_deref(),
            Some("Enriched technical content for candidate 3")
        );
        assert_eq!(cand_3.research.hook_origin.as_deref(), Some("verified_hook"));
    }

    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn test_save_candidates_error_propagation_on_write_failure() {
    let tmp = std::env::temp_dir().join(format!("test_dn_storage_err_{}", uuid::Uuid::new_v4()));
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
    let err = res.unwrap_err();
    assert!(
        err.contains("Failed to write candidate state")
            || err.contains("Failed to open lock file")
            || err.contains("Not a directory"),
        "Unexpected error: {err}"
    );

    let _ = fs::remove_file(&storage.state_dir);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_reservation_lifecycle_token_verification_and_published_invariance() {
    let tmp = std::env::temp_dir().join(format!("test_dn_res_life_{}", uuid::Uuid::new_v4()));
    let storage = FileStorageAdapter::new(&tmp);

    // 1. Reserve candidate with lease
    let token = storage
        .reserve_handoff("cand_life_01", 3600, "worker_alpha")
        .expect("Reservation must succeed");
    assert!(!token.is_empty());
    assert!(storage.is_consumed("cand_life_01"));

    // 2. Duplicate reservation while active lease must fail
    let dup_err = storage.reserve_handoff("cand_life_01", 3600, "worker_beta");
    assert!(dup_err.is_err());
    assert!(dup_err.unwrap_err().contains("actively reserved"));

    // 3. Release with invalid token must fail
    let rel_bad = storage.release_reservation("cand_life_01", "wrong_token");
    assert!(rel_bad.is_err());
    assert!(rel_bad.unwrap_err().contains("Invalid reservation token"));
    assert!(storage.is_consumed("cand_life_01"));

    // 4. Commit with invalid token must fail
    let com_bad = storage.commit_consumed("cand_life_01", "wrong_token");
    assert!(com_bad.is_err());

    // 5. Commit with valid token transitions to PUBLISHED
    let com_ok = storage.commit_consumed("cand_life_01", &token);
    assert!(com_ok.is_ok());
    assert!(storage.is_consumed("cand_life_01"));

    // 6. Once PUBLISHED, release must be rejected!
    let rel_pub = storage.release_reservation("cand_life_01", &token);
    assert!(rel_pub.is_err());
    assert!(rel_pub.unwrap_err().contains("PUBLISHED"));

    // 7. Restart adapter and confirm PUBLISHED status is recovered from disk
    {
        let restarted = FileStorageAdapter::new(&tmp);
        assert!(restarted.is_consumed("cand_life_01"));
        let rel_restart = restarted.release_reservation("cand_life_01", &token);
        assert!(rel_restart.is_err());
        assert!(rel_restart.unwrap_err().contains("PUBLISHED"));
    }

    let _ = fs::remove_dir_all(&tmp);
}
