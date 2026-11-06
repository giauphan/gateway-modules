//! SQLite transactional storage adapter with ACID guarantees, WAL journaling, and legacy JSON migration.
//!
//! Enforces Clean Architecture: implements StoragePort, guarantees atomic reserve/commit/release
//! across concurrent worker processes, and survives simulated power loss/crashes without state loss.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json;

use crate::domain::{HandoffReservation, ModuleConfig, NormalizedCandidate, ReservationStatus};
use crate::ports::StoragePort;
use super::file::resolve_state_dir;
use super::merge::merge_candidate_entry;

pub struct SqliteStorageAdapter {
    state_dir: PathBuf,
    db_path: PathBuf,
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorageAdapter {
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Result<Self, String> {
        let state_dir = resolve_state_dir(base_dir.as_ref());
        let _ = fs::create_dir_all(&state_dir);
        let db_path = state_dir.join("daily_news.db");

        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Failed to open SQLite database at {db_path:?}: {e}"))?;

        // Initialize pragmas for high concurrency and crash durability
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 10000;
             PRAGMA foreign_keys = ON;"
        ).map_err(|e| format!("Failed to configure SQLite pragmas: {e}"))?;

        // Initialize schema
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                json_val TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS candidates (
                candidate_id TEXT PRIMARY KEY,
                json_val TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS consumed (
                candidate_id TEXT PRIMARY KEY,
                consumed_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS reservations (
                candidate_id TEXT PRIMARY KEY,
                reservation_id TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                worker_id TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS idempotency_keys (
                key TEXT PRIMARY KEY,
                candidate_id TEXT NOT NULL,
                action TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );"
        ).map_err(|e| format!("Failed to initialize SQLite schema: {e}"))?;

        let adapter = Self {
            state_dir: state_dir.clone(),
            db_path,
            conn: Arc::new(Mutex::new(conn)),
        };

        // Run automatic migration from legacy JSON files
        adapter.migrate_legacy_json_files()?;

        Ok(adapter)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn migrate_legacy_json_files(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("Failed to start migration transaction: {e}"))?;

        // 1. Migrate config.json
        let config_path = self.state_dir.join("config.json");
        if config_path.is_file() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                let count: i64 = tx.query_row("SELECT count(*) FROM config WHERE key = 'active'", [], |r| r.get(0)).unwrap_or(0);
                if count == 0 {
                    let _ = tx.execute("INSERT OR REPLACE INTO config (key, json_val) VALUES ('active', ?)", params![content]);
                }
            }
        }

        // 2. Migrate candidates.json
        let candidates_path = self.state_dir.join("candidates.json");
        if candidates_path.is_file() {
            if let Ok(content) = fs::read_to_string(&candidates_path) {
                if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, NormalizedCandidate>>(&content) {
                    for (cid, cand) in map {
                        if let Ok(json) = serde_json::to_string(&cand) {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64;
                            let _ = tx.execute(
                                "INSERT OR IGNORE INTO candidates (candidate_id, json_val, updated_at) VALUES (?, ?, ?)",
                                params![cid, json, now],
                            );
                        }
                    }
                }
            }
        }

        // 3. Migrate consumed.json
        let consumed_path = self.state_dir.join("consumed.json");
        if consumed_path.is_file() {
            if let Ok(content) = fs::read_to_string(&consumed_path) {
                if let Ok(set) = serde_json::from_str::<HashSet<String>>(&content) {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    for cid in set {
                        let _ = tx.execute(
                            "INSERT OR IGNORE INTO consumed (candidate_id, consumed_at) VALUES (?, ?)",
                            params![cid, now],
                        );
                    }
                }
            }
        }

        // 4. Migrate reservations.json
        let reservations_path = self.state_dir.join("reservations.json");
        if reservations_path.is_file() {
            if let Ok(content) = fs::read_to_string(&reservations_path) {
                if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, HandoffReservation>>(&content) {
                    for (cid, res) in map {
                        let status_str = match res.status {
                            ReservationStatus::Reserved => "RESERVED",
                            ReservationStatus::Published => "PUBLISHED",
                        };
                        let _ = tx.execute(
                            "INSERT OR REPLACE INTO reservations (candidate_id, reservation_id, token_hash, worker_id, status, created_at, expires_at)
                             VALUES (?, ?, ?, ?, ?, ?, ?)",
                            params![
                                cid,
                                res.reservation_id,
                                res.token_hash,
                                res.worker_id,
                                status_str,
                                res.created_at,
                                res.expires_at as i64,
                            ],
                        );
                    }
                }
            }
        }

        // 5. Reconcile commit_tx.journal if present
        let journal_path = self.state_dir.join("commit_tx.journal");
        if journal_path.is_file() {
            if let Ok(content) = fs::read_to_string(&journal_path) {
                if let Ok(entry) = serde_json::from_str::<super::file::CommitJournalEntry>(&content) {
                    let now = entry.timestamp as i64;
                    let _ = tx.execute(
                        "INSERT OR IGNORE INTO consumed (candidate_id, consumed_at) VALUES (?, ?)",
                        params![entry.candidate_id, now],
                    );
                    let _ = tx.execute(
                        "UPDATE reservations SET status = 'PUBLISHED' WHERE candidate_id = ?",
                        params![entry.candidate_id],
                    );
                }
            }
            let _ = fs::remove_file(&journal_path);
        }

        tx.commit().map_err(|e| format!("Failed to commit migration: {e}"))?;
        Ok(())
    }
}

impl StoragePort for SqliteStorageAdapter {
    fn is_consumed(&self, candidate_id: &str) -> bool {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let count: Result<i64, _> = conn.query_row(
            "SELECT count(*) FROM consumed WHERE candidate_id = ?",
            params![candidate_id],
            |r| r.get(0),
        );
        count.unwrap_or(0) > 0
    }

    fn mark_consumed(&self, candidate_ids: &[String]) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("Failed to start transaction: {e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut inserted = 0;
        for cid in candidate_ids {
            let res = tx.execute(
                "INSERT OR IGNORE INTO consumed (candidate_id, consumed_at) VALUES (?, ?)",
                params![cid, now],
            );
            if let Ok(rows) = res {
                inserted += rows;
            }
        }
        tx.commit().map_err(|e| format!("Failed to commit mark_consumed: {e}"))?;
        Ok(inserted)
    }

    fn consumed_count(&self) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        let count: Result<i64, _> = conn.query_row(
            "SELECT count(*) FROM consumed",
            [],
            |r| r.get(0),
        );
        count.unwrap_or(0) as usize
    }

    fn load_config(&self) -> ModuleConfig {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return ModuleConfig::default(),
        };
        let json_res: Result<String, _> = conn.query_row(
            "SELECT json_val FROM config WHERE key = 'active'",
            [],
            |r| r.get(0),
        );
        if let Ok(json) = json_res {
            serde_json::from_str(&json).unwrap_or_default()
        } else {
            ModuleConfig::default()
        }
    }

    fn save_config(&self, config: &ModuleConfig) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO config (key, json_val) VALUES ('active', ?)",
            params![json],
        ).map_err(|e| format!("Failed to save config in SQLite: {e}"))?;
        Ok(())
    }

    fn cache_candidates(&self, candidates: &[NormalizedCandidate]) {
        let _ = self.save_candidates(candidates);
    }

    fn get_cached_candidates(&self) -> Vec<NormalizedCandidate> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare("SELECT json_val FROM candidates ORDER BY updated_at DESC") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        });
        let mut result = Vec::new();
        if let Ok(iter) = rows {
            for row in iter.flatten() {
                if let Ok(cand) = serde_json::from_str::<NormalizedCandidate>(&row) {
                    result.push(cand);
                }
            }
        }
        result
    }

    fn upsert_candidates(&self, candidates: &[NormalizedCandidate]) {
        let _ = self.save_candidates(candidates);
    }

    fn save_candidates(&self, candidates: &[NormalizedCandidate]) -> Result<usize, String> {
        let mut conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("Failed to start save transaction: {e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut saved = 0;
        for c in candidates {
            let existing_json: Option<String> = tx.query_row(
                "SELECT json_val FROM candidates WHERE candidate_id = ?",
                params![c.candidate_id],
                |r| r.get(0),
            ).ok();

            let merged = if let Some(ej) = existing_json {
                if let Ok(mut ex) = serde_json::from_str::<NormalizedCandidate>(&ej) {
                    merge_candidate_entry(&mut ex, c);
                    ex
                } else {
                    c.clone()
                }
            } else {
                c.clone()
            };

            let json = serde_json::to_string(&merged)
                .map_err(|e| format!("Failed to serialize candidate: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO candidates (candidate_id, json_val, updated_at) VALUES (?, ?, ?)",
                params![merged.candidate_id, json, now],
            ).map_err(|e| format!("Failed to upsert candidate {}: {e}", merged.candidate_id))?;
            saved += 1;
        }

        tx.commit().map_err(|e| format!("Failed to commit candidates: {e}"))?;
        Ok(saved)
    }

    fn get_candidate(&self, candidate_id: &str) -> Option<NormalizedCandidate> {
        let conn = self.conn.lock().ok()?;
        let json_res: Result<String, _> = conn.query_row(
            "SELECT json_val FROM candidates WHERE candidate_id = ?",
            params![candidate_id],
            |r| r.get(0),
        );
        let json = json_res.ok()?;
        serde_json::from_str(&json).ok()
    }

    fn reserve_handoff(
        &self,
        candidate_id: &str,
        lease_seconds: u64,
        worker_id: &str,
    ) -> Result<String, String> {
        let mut conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("Failed to start reservation transaction: {e}"))?;

        // 1. Verify candidate exists
        let exists: i64 = tx.query_row(
            "SELECT count(*) FROM candidates WHERE candidate_id = ?",
            params![candidate_id],
            |r| r.get(0),
        ).map_err(|e| format!("Failed to query candidate existence: {e}"))?;
        if exists == 0 {
            return Err(format!("Candidate '{candidate_id}' does not exist"));
        }

        // 2. Verify candidate is not consumed
        let is_consumed: i64 = tx.query_row(
            "SELECT count(*) FROM consumed WHERE candidate_id = ?",
            params![candidate_id],
            |r| r.get(0),
        ).map_err(|e| format!("Failed to query consumed status: {e}"))?;
        if is_consumed > 0 {
            return Err(format!("Candidate '{candidate_id}' is already consumed"));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 3. Check existing reservation
        let existing: Result<(String, String, u64), _> = tx.query_row(
            "SELECT status, worker_id, expires_at FROM reservations WHERE candidate_id = ?",
            params![candidate_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64)),
        );

        if let Ok((status, owner, expires_at)) = existing {
            if status == "PUBLISHED" {
                return Err(format!("Candidate '{candidate_id}' is already PUBLISHED"));
            }
            if status == "RESERVED" && now < expires_at {
                return Err(format!(
                    "Candidate '{candidate_id}' is actively reserved by worker '{owner}' until {expires_at}"
                ));
            }
        }

        let token = format!("claim_{}_{}", uuid::Uuid::new_v4(), candidate_id);
        let token_hash = crate::url::compute_sha256(&token);
        let res_id = format!("res_{}", uuid::Uuid::new_v4());
        let created_at = crate::date::current_utc_timestamp();
        let expires_at = now.saturating_add(lease_seconds);

        tx.execute(
            "INSERT OR REPLACE INTO reservations (candidate_id, reservation_id, token_hash, worker_id, status, created_at, expires_at)
             VALUES (?, ?, ?, ?, 'RESERVED', ?, ?)",
            params![
                candidate_id,
                res_id,
                token_hash,
                worker_id,
                created_at,
                expires_at as i64,
            ],
        ).map_err(|e| format!("Failed to record reservation in SQLite: {e}"))?;

        tx.commit().map_err(|e| format!("Failed to commit reservation: {e}"))?;
        Ok(token)
    }

    fn commit_consumed(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("Failed to start commit transaction: {e}"))?;

        let res_info: Result<(String, String, u64), _> = tx.query_row(
            "SELECT token_hash, status, expires_at FROM reservations WHERE candidate_id = ?",
            params![candidate_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64)),
        );

        let (stored_token_hash, status, expires_at) = match res_info {
            Ok(info) => info,
            Err(_) => return Err(format!("No reservation found for candidate '{candidate_id}'")),
        };

        let token_hash = crate::url::compute_sha256(token);
        if stored_token_hash != token_hash {
            return Err(format!("Invalid reservation token for candidate '{candidate_id}'"));
        }

        if status == "PUBLISHED" {
            return Ok(true);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now > expires_at {
            return Err(format!("Reservation lease for candidate '{candidate_id}' has expired"));
        }

        tx.execute(
            "UPDATE reservations SET status = 'PUBLISHED' WHERE candidate_id = ?",
            params![candidate_id],
        ).map_err(|e| format!("Failed to update reservation to PUBLISHED: {e}"))?;

        tx.execute(
            "INSERT OR IGNORE INTO consumed (candidate_id, consumed_at) VALUES (?, ?)",
            params![candidate_id, now as i64],
        ).map_err(|e| format!("Failed to mark candidate consumed in SQLite: {e}"))?;

        tx.commit().map_err(|e| format!("Failed to commit transaction: {e}"))?;
        Ok(true)
    }

    fn release_reservation(&self, candidate_id: &str, token: &str) -> Result<bool, String> {
        let mut conn = self.conn.lock().map_err(|_| "Poisoned SQLite lock")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("Failed to start release transaction: {e}"))?;

        let res_info: Result<(String, String), _> = tx.query_row(
            "SELECT token_hash, status FROM reservations WHERE candidate_id = ?",
            params![candidate_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );

        let (stored_token_hash, status) = match res_info {
            Ok(info) => info,
            Err(_) => return Err(format!("No reservation found for candidate '{candidate_id}'")),
        };

        let token_hash = crate::url::compute_sha256(token);
        if stored_token_hash != token_hash {
            return Err(format!("Invalid reservation token for candidate '{candidate_id}'"));
        }

        if status == "PUBLISHED" {
            return Err(format!("Candidate '{candidate_id}' is already PUBLISHED"));
        }

        tx.execute(
            "DELETE FROM reservations WHERE candidate_id = ?",
            params![candidate_id],
        ).map_err(|e| format!("Failed to delete reservation in SQLite: {e}"))?;

        tx.execute(
            "DELETE FROM consumed WHERE candidate_id = ?",
            params![candidate_id],
        ).map_err(|e| format!("Failed to delete from consumed in SQLite: {e}"))?;

        tx.commit().map_err(|e| format!("Failed to commit release: {e}"))?;
        Ok(true)
    }
}
